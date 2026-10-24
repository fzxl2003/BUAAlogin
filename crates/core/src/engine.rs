pub use crate::recovery::RecoveryMode;
use crate::{
    cancellation::Cancellation,
    connectivity::{self, Authentication, Connectivity, ProbeReport},
    interfaces::{self, Interface, InterfaceRecord},
    recovery::{self, RecoveryBook, RecoveryOps},
    transport::{AuthError, AuthErrorKind, Portal, Session},
    Config, DashboardSnapshot, Event, TaskScheduleReason, TaskSnapshot, TaskSnapshots,
};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestKind {
    PortalAuth,
    Connectivity,
    Browser,
}

#[derive(Clone, Copy)]
pub enum Mode {
    Monitor,
    Inspect,
    Status,
    Once,
    Test(TestKind),
}
#[derive(Clone)]
enum Command {
    Stop,
    Network(Vec<String>),
    Suspend,
    Resume,
    Tick,
    Recover(RecoveryMode),
    Test(TestKind),
}
#[derive(Clone)]
pub struct Signal {
    tx: Sender<Command>,
    generation: Arc<AtomicU64>,
}
impl Signal {
    fn send(&self, command: Command, cancel: bool) {
        if cancel {
            self.generation.fetch_add(1, Ordering::SeqCst);
        }
        let _ = self.tx.send(command);
    }
    pub fn stop(&self) {
        self.send(Command::Stop, true);
    }
    pub fn network_changed(&self) {
        self.network_changed_for(vec![]);
    }
    pub fn network_changed_for(&self, names: Vec<String>) {
        self.send(Command::Network(names), true);
    }
    pub fn suspend(&self) {
        self.send(Command::Suspend, true);
    }
    pub fn resume(&self) {
        self.send(Command::Resume, true);
    }
    pub fn tick(&self) {
        self.send(Command::Tick, false);
    }
    pub fn recover(&self, mode: RecoveryMode) {
        self.send(Command::Recover(mode), false);
    }
    pub fn test(&self, kind: TestKind) {
        self.send(Command::Test(kind), false);
    }
}
pub struct Handle {
    pub signal: Signal,
    worker: Option<JoinHandle<()>>,
}
impl Handle {
    pub fn spawn(
        config: Config,
        mode: Mode,
        emit: impl Fn(Event) + Send + 'static,
    ) -> Result<Self, String> {
        config.validate(matches!(
            mode,
            Mode::Monitor | Mode::Once | Mode::Test(TestKind::PortalAuth | TestKind::Browser)
        ))?;
        let (tx, rx) = mpsc::channel();
        let generation = Arc::new(AtomicU64::new(0));
        let signal = Signal {
            tx,
            generation: generation.clone(),
        };
        let worker_signal = signal.clone();
        let worker = thread::Builder::new()
            .name("campus-monitor".into())
            .spawn(move || {
                let interval = config.interval;
                // Keep notification registrations alive only while this worker exists.
                let _watcher = if matches!(mode, Mode::Monitor) {
                    interfaces::watch(worker_signal)
                } else {
                    None
                };
                if matches!(mode, Mode::Monitor) && _watcher.is_none() {
                    emit(Event::new("failed", "无法订阅网络变化通知，监控已停止"));
                    emit_dashboard(&emit, interval, "stopped", "已停止", None);
                    emit(Event::new("stopped", "已停止"));
                    return;
                }
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run(config, mode, generation, rx, &emit)
                }));
                if result.is_err() {
                    emit(Event::new("failed", "监控内部异常，已停止"));
                }
                if matches!(mode, Mode::Monitor) {
                    emit_dashboard(&emit, interval, "stopped", "已停止", None);
                }
                emit(Event::new("stopped", "已停止"));
            })
            .map_err(|_| "无法启动监控线程")?;
        Ok(Self {
            signal,
            worker: Some(worker),
        })
    }
    pub fn is_finished(&self) -> bool {
        self.worker.as_ref().is_none_or(|w| w.is_finished())
    }
    pub fn join(&mut self) {
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        self.signal.stop();
        self.join();
    }
}

struct Slot {
    session: Session,
    failures: u32,
    due: Instant,
    online: bool,
    reported: Option<String>,
    last_check_at: Option<u64>,
    retry_at: Option<u64>,
    message: String,
    authentication: Authentication,
    connectivity: Connectivity,
    recovery_stage: Option<String>,
    auth_last_run_at: Option<u64>,
    auth_next_run_at: Option<u64>,
    auth_phase: String,
    auth_message: String,
    connectivity_last_run_at: Option<u64>,
    connectivity_next_run_at: Option<u64>,
    connectivity_phase: String,
    connectivity_message: String,
    browser_last_run_at: Option<u64>,
    browser_next_run_at: Option<u64>,
    browser_phase: String,
    browser_message: String,
    auth_enabled: bool,
    connectivity_enabled: bool,
    browser_enabled: bool,
}
fn task_snapshot(
    enabled: bool,
    phase: &str,
    message: &str,
    last: Option<u64>,
    next: Option<u64>,
    browser: bool,
) -> TaskSnapshot {
    TaskSnapshot {
        enabled,
        phase: phase.into(),
        message: message.into(),
        last_run_at: last,
        next_run_at: next,
        schedule_reason: if !enabled {
            TaskScheduleReason::Disabled
        } else if phase == "cooldown" {
            TaskScheduleReason::Cooldown
        } else if browser && next.is_none() {
            TaskScheduleReason::WaitingTrigger
        } else {
            TaskScheduleReason::Scheduled
        },
    }
}
fn task_snapshots(slot: &Slot) -> TaskSnapshots {
    TaskSnapshots {
        portal_auth: task_snapshot(
            slot.auth_enabled,
            &slot.auth_phase,
            &slot.auth_message,
            slot.auth_last_run_at,
            slot.auth_next_run_at,
            false,
        ),
        connectivity: task_snapshot(
            slot.connectivity_enabled,
            &slot.connectivity_phase,
            &slot.connectivity_message,
            slot.connectivity_last_run_at,
            slot.connectivity_next_run_at,
            false,
        ),
        browser_fallback: task_snapshot(
            slot.browser_enabled,
            &slot.browser_phase,
            &slot.browser_message,
            slot.browser_last_run_at,
            slot.browser_next_run_at,
            true,
        ),
    }
}
pub fn retry_delay(failures: u32) -> u64 {
    (30u64 << failures.saturating_sub(1).min(4)).min(300)
}
fn emit_dashboard(
    emit: &dyn Fn(Event),
    interval: u64,
    phase: &str,
    message: &str,
    current: Option<(&Interface, &Slot)>,
) {
    let tasks = if let Some((_, slot)) = current {
        task_snapshots(slot)
    } else {
        TaskSnapshots {
            portal_auth: TaskSnapshot {
                enabled: false,
                phase: "idle".into(),
                message: "未启动".into(),
                last_run_at: None,
                next_run_at: None,
                schedule_reason: TaskScheduleReason::OnStart,
            },
            connectivity: TaskSnapshot {
                enabled: false,
                phase: "idle".into(),
                message: "未启动".into(),
                last_run_at: None,
                next_run_at: None,
                schedule_reason: TaskScheduleReason::OnStart,
            },
            browser_fallback: TaskSnapshot {
                enabled: false,
                phase: "idle".into(),
                message: "等待外网异常".into(),
                last_run_at: None,
                next_run_at: None,
                schedule_reason: TaskScheduleReason::WaitingTrigger,
            },
        }
    };
    let mut event = Event::new("dashboard", message);
    event.dashboard = Some(DashboardSnapshot {
        phase: phase.into(),
        interface: current.map(|(i, _)| i.name.clone()),
        local_ip: current.map(|(i, _)| i.address),
        campus_ip: current.and_then(|(_, s)| s.session.campus_ip().map(str::to_owned)),
        last_check_at: current.and_then(|(_, s)| s.last_check_at),
        retry_at: if matches!(phase, "retry" | "suspect" | "cooldown") {
            current.and_then(|(_, s)| s.retry_at)
        } else {
            None
        },
        interval,
        authentication: current.map_or(Authentication::Unknown, |(_, slot)| slot.authentication),
        connectivity: current.map_or(Connectivity::Unknown, |(_, slot)| slot.connectivity),
        recovery_stage: current.and_then(|(_, slot)| slot.recovery_stage.clone()),
        tasks,
    });
    emit(event);
}
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn emit_safe(emit: &dyn Fn(Event), config: &Config, mut event: Event) {
    for secret in [config.password.as_str(), config.username.as_str()] {
        if !secret.is_empty() {
            event.message = event.message.replace(secret, "[redacted]");
        }
    }
    emit(event);
}
fn refresh(
    config: &Config,
    slots: &mut HashMap<Interface, Slot>,
    generation: &Arc<AtomicU64>,
    emit: &dyn Fn(Event),
) {
    match interfaces::discover(config.try_all) {
        Ok(mut records) => {
            records.retain(|i| i.selected(&config.interface));
            slots.retain(|i, _| records.contains(i));
            for i in records {
                if slots.contains_key(&i) {
                    continue;
                }
                match Session::new(i.clone(), config, generation.clone()) {
                    Ok(session) => {
                        slots.insert(
                            i,
                            Slot {
                                session,
                                failures: 0,
                                due: Instant::now(),
                                online: false,
                                reported: None,
                                last_check_at: None,
                                retry_at: None,
                                message: "正在检查校园网连接…".into(),
                                authentication: Authentication::Unknown,
                                connectivity: Connectivity::Unknown,
                                recovery_stage: None,
                                auth_last_run_at: None,
                                auth_next_run_at: Some(unix_now()),
                                auth_phase: if config.portal_auth_enabled {
                                    "pending"
                                } else {
                                    "disabled"
                                }
                                .into(),
                                auth_message: if config.portal_auth_enabled {
                                    "等待运行"
                                } else {
                                    "已关闭"
                                }
                                .into(),
                                connectivity_last_run_at: None,
                                connectivity_next_run_at: Some(unix_now()),
                                connectivity_phase: if config.connectivity_probe_enabled {
                                    "pending"
                                } else {
                                    "disabled"
                                }
                                .into(),
                                connectivity_message: if config.connectivity_probe_enabled {
                                    "等待运行"
                                } else {
                                    "已关闭"
                                }
                                .into(),
                                browser_last_run_at: None,
                                browser_next_run_at: None,
                                browser_phase: if config.browser_fallback {
                                    "waiting"
                                } else {
                                    "disabled"
                                }
                                .into(),
                                browser_message: if config.browser_fallback {
                                    "等待外网异常"
                                } else {
                                    "已关闭"
                                }
                                .into(),
                                auth_enabled: config.portal_auth_enabled,
                                connectivity_enabled: config.connectivity_probe_enabled,
                                browser_enabled: config.browser_fallback
                                    && config.connectivity_probe_enabled,
                            },
                        );
                    }
                    Err(e) => emit_safe(emit, config, Event::new("error", e)),
                }
            }
        }
        Err(e) => {
            slots.clear();
            emit_safe(emit, config, Event::new("error", e));
        }
    }
}
fn run(
    config: Config,
    mode: Mode,
    generation: Arc<AtomicU64>,
    rx: Receiver<Command>,
    emit: &dyn Fn(Event),
) {
    if !matches!(mode, Mode::Monitor) {
        let start_generation = generation.load(Ordering::SeqCst);
        let candidates = match interfaces::discover(config.try_all) {
            Ok(v) => v
                .into_iter()
                .filter(|i| i.selected(&config.interface))
                .collect::<Vec<_>>(),
            Err(e) => {
                emit_safe(emit, &config, Event::new("error", e));
                return;
            }
        };
        let mut records = vec![];
        let mut success = false;
        for interface in candidates {
            if generation.load(Ordering::SeqCst) != start_generation {
                break;
            }
            let mut session = match Session::new(interface.clone(), &config, generation.clone()) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let result = match mode {
                Mode::Inspect => session.inspect().map(|_| true),
                Mode::Status | Mode::Once => {
                    check_once(&mut session, &config, matches!(mode, Mode::Once), emit)
                }
                Mode::Test(kind) => test_once(&mut session, &config, kind, emit),
                _ => unreachable!(),
            };
            let campus = result.as_ref().copied().unwrap_or(false);
            records.push(InterfaceRecord {
                binding: interface.binding(),
                label: format!("{} · {}", interface.name, interface.address),
                campus_ip: session.campus_ip().map(str::to_owned),
                interface,
                campus,
                message: if campus {
                    "校园网".into()
                } else {
                    "未检测到校园网".into()
                },
            });
            success |= campus;
            if !matches!(mode, Mode::Inspect | Mode::Test(_)) {
                emit_safe(
                    emit,
                    &config,
                    Event::new(
                        if matches!(mode, Mode::Test(_)) {
                            "test_result"
                        } else if campus {
                            "online"
                        } else {
                            "error"
                        },
                        result.err().unwrap_or_else(|| {
                            if campus {
                                "校园网在线".into()
                            } else {
                                "校园网未登录".into()
                            }
                        }),
                    ),
                );
                if campus && !config.try_all {
                    break;
                }
            }
        }
        if matches!(mode, Mode::Inspect) {
            let mut event = Event::new("interfaces", "接口检测完成");
            event.records = Some(records);
            emit(event);
        } else {
            emit(Event::new(
                if success { "complete" } else { "failed" },
                if success {
                    "操作成功"
                } else {
                    "没有在线校园网接口"
                },
            ));
        }
        return;
    }
    let mut slots = HashMap::new();
    refresh(&config, &mut slots, &generation, emit);
    let mut preferred: Option<Interface> = None;
    let mut controls = crate::scheduler::Controls::default();
    let mut last = String::new();
    let mut recoveries = RecoveryBook::default();
    let mut manual_recovery = None;
    let mut pending_test = None;
    loop {
        // Drain controls before starting any new request.
        while let Ok(command) = rx.try_recv() {
            if apply_control(
                command,
                &mut controls,
                &mut slots,
                &mut manual_recovery,
                &mut pending_test,
                config.interval,
                emit,
            ) {
                return;
            }
        }
        if let Some(changes) = controls.refresh(Instant::now()) {
            // Discard cookies/connections only for affected interfaces.
            slots.retain(|interface, _| !changes.affects(interface));
            refresh(&config, &mut slots, &generation, emit);
            last.clear();
            // An unrelated network notification must not erase the selected connection.
            let current = preferred
                .as_ref()
                .and_then(|i| slots.get(i).map(|s| (i, s)))
                .or_else(|| slots.iter().min_by_key(|(_, s)| (!s.online, s.due)));
            if let Some((interface, slot)) = current {
                let phase = if slot.online {
                    "online"
                } else if slot.last_check_at.is_some() {
                    "retry"
                } else {
                    "checking"
                };
                emit_dashboard(
                    &|e| emit_safe(emit, &config, e),
                    config.interval,
                    phase,
                    &slot.message,
                    Some((interface, slot)),
                );
            }
        }
        if !controls.suspended && controls.deadline.is_none() {
            let selected = preferred
                .as_ref()
                .filter(|p| slots.get(*p).is_some_and(|s| s.online))
                .cloned();
            let mut candidates: Vec<_> = slots.keys().cloned().collect();
            candidates.sort_by_key(|i| {
                (
                    Some(i) != preferred.as_ref(),
                    i.address.octets()[0] != 10,
                    i.name.clone(),
                    i.address,
                )
            });
            if !config.try_all {
                if let Some(p) = selected {
                    candidates.retain(|i| i == &p);
                }
            }
            for interface in candidates {
                let slot = slots.get_mut(&interface).unwrap();
                if slot.due > Instant::now() {
                    continue;
                }
                let active = generation.load(Ordering::SeqCst);
                emit_dashboard(
                    emit,
                    config.interval,
                    "checking",
                    "正在检查校园网连接…",
                    Some((&interface, slot)),
                );
                slot.session.activate();
                let requested_test = pending_test.take();
                let result = check_connection(
                    slot,
                    &interface,
                    &config,
                    &mut recoveries,
                    manual_recovery.take(),
                    requested_test,
                    emit,
                );
                if let Some(kind) = requested_test {
                    let message = match &result {
                        Ok((_, message, _)) => message.clone(),
                        Err(error) => error.to_string(),
                    };
                    emit_safe(
                        emit,
                        &config,
                        Event::new("test_result", format!("{:?} 测试完成：{}", kind, message)),
                    );
                }
                if generation.load(Ordering::SeqCst) != active {
                    break;
                }
                slot.last_check_at = Some(unix_now());
                let mut event = match result {
                    Ok((phase, message, delay)) => {
                        slot.failures = 0;
                        slot.online = phase == "online";
                        slot.retry_at = if matches!(phase, "suspect" | "cooldown") {
                            slot.retry_at.or(Some(unix_now() + delay))
                        } else {
                            None
                        };
                        let next = unix_now() + delay;
                        match requested_test {
                            Some(TestKind::PortalAuth) => slot.auth_next_run_at = Some(next),
                            Some(TestKind::Connectivity) => {
                                slot.connectivity_next_run_at = Some(next)
                            }
                            Some(TestKind::Browser) => slot.browser_next_run_at = None,
                            None => {
                                if slot.auth_enabled {
                                    slot.auth_next_run_at = Some(next);
                                }
                                if slot.connectivity_enabled {
                                    slot.connectivity_next_run_at = Some(next);
                                }
                            }
                        }
                        let earliest = [slot.auth_next_run_at, slot.connectivity_next_run_at]
                            .into_iter()
                            .flatten()
                            .min()
                            .unwrap_or(next);
                        slot.due = Instant::now()
                            + Duration::from_secs(earliest.saturating_sub(unix_now()));
                        if slot.online {
                            preferred = Some(interface.clone());
                        }
                        Event::new(phase, message)
                    }
                    Err(e) => {
                        slot.failures = slot.failures.saturating_add(1);
                        slot.online = false;
                        let delay = retry_delay(slot.failures);
                        slot.due = Instant::now() + Duration::from_secs(delay);
                        let mut event = Event::new(
                            "retry",
                            format!("{}：{}；{} 秒后自动重试", interface.name, e, delay),
                        );
                        slot.retry_at = Some(unix_now() + delay);
                        match requested_test {
                            Some(TestKind::PortalAuth) => slot.auth_next_run_at = slot.retry_at,
                            Some(TestKind::Connectivity) => {
                                slot.connectivity_next_run_at = slot.retry_at
                            }
                            Some(TestKind::Browser) => slot.browser_next_run_at = slot.retry_at,
                            None => {
                                if slot.auth_enabled {
                                    slot.auth_next_run_at = slot.retry_at;
                                }
                                if slot.connectivity_enabled {
                                    slot.connectivity_next_run_at = slot.retry_at;
                                }
                            }
                        }
                        event.retry_at = slot.retry_at;
                        event
                    }
                };
                event.authentication = Some(slot.authentication);
                event.connectivity = Some(slot.connectivity);
                slot.message = if slot.online {
                    event.message.clone()
                } else {
                    event.message.clone()
                };
                let phase = event.state.as_str();
                emit_dashboard(
                    &|e| emit_safe(emit, &config, e),
                    config.interval,
                    phase,
                    &slot.message,
                    Some((&interface, slot)),
                );
                last.clear();
                let key = format!("{}:{}", event.state, event.message);
                // Each failure has a new retry deadline, even if the error is unchanged.
                if event.state == "retry" || slot.reported.as_ref() != Some(&key) {
                    emit_safe(emit, &config, event);
                    slot.reported = Some(key);
                }
                if slot.online && !config.try_all {
                    break;
                }
            }
        }
        let preferred_online = preferred
            .as_ref()
            .filter(|p| slots.get(*p).is_some_and(|s| s.online));
        let deadline = if controls.suspended {
            None
        } else if let Some(d) = controls.deadline {
            Some(d)
        } else {
            slots
                .iter()
                .filter(|(i, _)| {
                    config.try_all || preferred_online.is_none() || preferred_online == Some(*i)
                })
                .map(|(_, s)| s.due)
                .min()
        };
        if slots.is_empty() && controls.deadline.is_none() {
            let message = if controls.suspended {
                "睡眠期间暂停"
            } else {
                "等待可用网络接口"
            };
            if last != message {
                emit_dashboard(
                    emit,
                    config.interval,
                    if controls.suspended {
                        "suspended"
                    } else {
                        "waiting_network"
                    },
                    message,
                    None,
                );
                emit(Event::new("waiting_network", message));
                last = message.into();
            }
        }
        let wait = deadline.map(|d| d.saturating_duration_since(Instant::now()));
        if config.external_timer {
            let mut event = Event::new("schedule", "");
            event.delay = wait.map(|d| d.as_nanos().div_ceil(1_000_000_000) as u64);
            event.leeway = if controls.deadline.is_none() && slots.values().all(|s| s.online) {
                (config.interval / 10).min(5)
            } else {
                0
            };
            emit(event);
        }
        let command = match (config.external_timer, wait) {
            (false, Some(wait)) => match rx.recv_timeout(wait) {
                Ok(c) => Some(c),
                Err(RecvTimeoutError::Timeout) => Some(Command::Tick),
                Err(_) => None,
            },
            _ => rx.recv().ok(),
        };
        match command {
            Some(c) => {
                if apply_control(
                    c,
                    &mut controls,
                    &mut slots,
                    &mut manual_recovery,
                    &mut pending_test,
                    config.interval,
                    emit,
                ) {
                    return;
                }
            }
            None => return,
        }
    }
}
fn apply_control(
    command: Command,
    controls: &mut crate::scheduler::Controls,
    slots: &mut HashMap<Interface, Slot>,
    manual_recovery: &mut Option<RecoveryMode>,
    pending_test: &mut Option<TestKind>,
    interval: u64,
    emit: &dyn Fn(Event),
) -> bool {
    match command {
        Command::Stop => return true,
        Command::Suspend => {
            emit_dashboard(emit, interval, "suspended", "睡眠期间暂停", None);
            controls.suspend();
            slots.clear();
            *manual_recovery = None;
            *pending_test = None;
        }
        Command::Network(names) => controls.network(Instant::now(), names),
        Command::Resume => controls.resume(Instant::now()),
        Command::Tick => {}
        Command::Recover(mode) => {
            if !controls.suspended {
                *manual_recovery = Some(mode);
                for slot in slots.values_mut() {
                    slot.due = Instant::now();
                }
                if slots.is_empty() {
                    *manual_recovery = None;
                    emit(Event::new("error", "无法修复：等待可用网络接口"));
                }
            }
        }
        Command::Test(kind) => {
            if !controls.suspended {
                *pending_test = Some(kind);
                for slot in slots.values_mut() {
                    slot.due = Instant::now();
                }
                if slots.is_empty() {
                    *pending_test = None;
                    emit(Event::new("error", "无法测试：等待可用网络接口"));
                }
            }
        }
    }
    false
}

struct SessionRecovery<'a> {
    session: &'a mut Session,
    config: &'a Config,
    cancel: Cancellation,
    authentication: &'a mut Authentication,
    connectivity: &'a mut Connectivity,
}
impl RecoveryOps for SessionRecovery<'_> {
    fn reset(&mut self) -> Result<(), AuthError> {
        let reset_cancel = self.cancel.with_timeout(Duration::from_secs(60));
        reset_cancel.check()?;
        self.session.logout(&self.config.username)?;
        *self.authentication = Authentication::Offline;
        self.wait(2)?;
        reset_cancel.check()?;
        self.session
            .force_login(&self.config.username, &self.config.password)?;
        reset_cancel.check()?;
        *self.authentication = Authentication::Online;
        Ok(())
    }
    fn probe(&mut self) -> Result<bool, AuthError> {
        self.cancel.check()?;
        let report = connectivity::probe(&self.session.interface, &self.cancel);
        self.cancel.check()?;
        *self.connectivity = report.connectivity;
        Ok(report.available())
    }
    fn browser(&mut self) -> Result<(), AuthError> {
        self.cancel = self.cancel.with_timeout(Duration::from_secs(90));
        self.cancel.check()?;
        #[cfg(feature = "browser-fallback")]
        {
            crate::browser::recover(&self.session.interface, self.config, &self.cancel)?;
        }
        #[cfg(not(feature = "browser-fallback"))]
        {
            return Err("当前构建不包含网页兜底".into());
        }
        #[cfg(feature = "browser-fallback")]
        {
            *self.authentication = Authentication::Unknown;
            Ok(())
        }
    }
    fn wait(&mut self, seconds: u64) -> Result<(), AuthError> {
        self.cancel
            .wait(Duration::from_secs(seconds))
            .map_err(Into::into)
    }
}
fn authentication(session: &mut Session) -> (Authentication, Option<String>) {
    let result = if session.campus_ip().is_none() {
        session.inspect().and_then(|_| session.online())
    } else {
        session.online()
    };
    match result {
        Ok(true) => (Authentication::Online, None),
        Ok(false) => (Authentication::Offline, None),
        Err(error) => (Authentication::Unknown, Some(error)),
    }
}
fn test_once(
    session: &mut Session,
    config: &Config,
    kind: TestKind,
    emit: &dyn Fn(Event),
) -> Result<bool, String> {
    let cancel = session.cancellation();
    match kind {
        TestKind::PortalAuth => {
            let (mut state, note) = authentication(session);
            if state == Authentication::Offline {
                session
                    .login(&config.username, &config.password)
                    .map_err(|e| e.to_string())?;
                state = Authentication::Online;
            }
            let mut event = Event::new(
                "test_result",
                format!(
                    "校园网认证测试：{}{}",
                    state.label(),
                    note.map(|v| format!("；{v}")).unwrap_or_default()
                ),
            );
            event.authentication = Some(state);
            emit_safe(emit, config, event);
            Ok(state == Authentication::Online)
        }
        TestKind::Connectivity => {
            let report = connectivity::probe(&session.interface, &cancel);
            let ok = report.available();
            let mut event = Event::new(
                "test_result",
                format!(
                    "外网检测测试：{}；{}",
                    report.connectivity.label(),
                    report.summary()
                ),
            );
            event.connectivity = Some(report.connectivity);
            event.probes = Some(report.results);
            emit_safe(emit, config, event);
            Ok(ok)
        }
        TestKind::Browser => {
            let mut auth = Authentication::Unknown;
            let mut internet = Connectivity::Unknown;
            let mut ops = SessionRecovery {
                session,
                config,
                cancel,
                authentication: &mut auth,
                connectivity: &mut internet,
            };
            recovery::execute(&mut ops, RecoveryMode::Browser, true, &|phase, message| {
                emit_safe(emit, config, Event::new(phase, message))
            })
            .map_err(|e| e.to_string())?;
            let mut event = Event::new("test_result", "网页兜底测试成功，外网已验证可用");
            event.connectivity = Some(internet);
            emit_safe(emit, config, event);
            Ok(true)
        }
    }
}
fn check_once(
    session: &mut Session,
    config: &Config,
    login: bool,
    emit: &dyn Fn(Event),
) -> Result<bool, String> {
    let cancel = session.cancellation();
    let (mut auth, note) = authentication(session);
    let mut report = connectivity::probe(&session.interface, &cancel);
    cancel.check()?;
    if login && !report.available() {
        match session.login(&config.username, &config.password) {
            Ok(_) => auth = Authentication::Online,
            Err(error)
                if matches!(
                    error.kind,
                    AuthErrorKind::NeedsUser | AuthErrorKind::Cancelled
                ) =>
            {
                return Err(error.to_string())
            }
            Err(error) => emit_safe(emit, config, Event::new("error", error.to_string())),
        }
        report = connectivity::probe(&session.interface, &cancel);
        if !report.available() {
            let mut internet = report.connectivity;
            let mut ops = SessionRecovery {
                session,
                config,
                cancel: cancel.clone(),
                authentication: &mut auth,
                connectivity: &mut internet,
            };
            recovery::execute(
                &mut ops,
                RecoveryMode::Protocol,
                false,
                &|phase, message| emit_safe(emit, config, Event::new(phase, message)),
            )
            .map_err(|error| error.to_string())?;
            report.connectivity = internet;
        }
    }
    cancel.check()?;
    let mut event = Event::new(
        "connectivity",
        format!(
            "认证状态：{}；外网状态：{}；{}{}",
            auth.label(),
            report.connectivity.label(),
            report.summary(),
            note.map(|note| format!("；{note}")).unwrap_or_default()
        ),
    );
    event.authentication = Some(auth);
    event.connectivity = Some(report.connectivity);
    let available = report.available();
    event.probes = Some(report.results);
    emit_safe(emit, config, event);
    Ok(available)
}
fn check_connection(
    slot: &mut Slot,
    interface: &Interface,
    config: &Config,
    book: &mut RecoveryBook,
    manual: Option<RecoveryMode>,
    test: Option<TestKind>,
    emit: &dyn Fn(Event),
) -> Result<(&'static str, String, u64), AuthError> {
    let cancel = slot.session.cancellation();
    slot.retry_at = None;
    let now = unix_now();
    if test.is_none() && config.portal_auth_enabled || matches!(test, Some(TestKind::PortalAuth)) {
        slot.auth_phase = "running".into();
        slot.auth_message = "正在检查校园网认证".into();
        let (auth, _) = authentication(&mut slot.session);
        slot.authentication = auth;
        if auth == Authentication::Offline {
            match slot.session.login(&config.username, &config.password) {
                Ok(_) => slot.authentication = Authentication::Online,
                Err(error) if error.kind == AuthErrorKind::NeedsUser => {
                    slot.auth_last_run_at = Some(now);
                    slot.auth_phase = "failure".into();
                    slot.auth_message = error.to_string();
                    return Ok(("needs_user", error.to_string(), config.interval));
                }
                Err(error) => return Err(error),
            }
        }
        slot.auth_last_run_at = Some(now);
        slot.auth_next_run_at = Some(now + config.interval);
        slot.auth_phase = if slot.authentication == Authentication::Online {
            "success"
        } else {
            "failure"
        }
        .into();
        slot.auth_message = format!("认证状态：{}", slot.authentication.label());
    } else if test.is_none() {
        slot.authentication = Authentication::Unknown;
        slot.auth_next_run_at = None;
    }
    if matches!(test, Some(TestKind::PortalAuth)) {
        return Ok((
            if slot.authentication == Authentication::Online {
                "online"
            } else {
                "retry"
            },
            slot.auth_message.clone(),
            config.interval,
        ));
    }
    if matches!(test, Some(TestKind::Browser)) {
        slot.browser_phase = "running".into();
        slot.browser_message = "正在运行网页兜底测试".into();
        let mut auth = slot.authentication;
        let mut internet = slot.connectivity;
        let mut ops = SessionRecovery {
            session: &mut slot.session,
            config,
            cancel: cancel.clone(),
            authentication: &mut auth,
            connectivity: &mut internet,
        };
        recovery::execute(&mut ops, RecoveryMode::Browser, true, &|phase, message| {
            emit_safe(emit, config, Event::new(phase, message))
        })?;
        let completed = unix_now();
        slot.authentication = auth;
        slot.connectivity = internet;
        slot.browser_last_run_at = Some(completed);
        slot.browser_next_run_at = None;
        slot.browser_phase = "success".into();
        slot.browser_message = "网页兜底测试成功".into();
        return Ok((
            "online",
            "网页兜底测试成功，外网已验证可用".into(),
            config.interval,
        ));
    }
    if !config.connectivity_probe_enabled && test.is_none() {
        slot.connectivity = Connectivity::Unknown;
        slot.connectivity_next_run_at = None;
        return Ok((
            if slot.authentication == Authentication::Online {
                "online"
            } else {
                "checking"
            },
            if config.portal_auth_enabled {
                slot.auth_message.clone()
            } else {
                "未启用自动连接任务".into()
            },
            config.interval,
        ));
    }
    slot.connectivity_phase = "running".into();
    slot.connectivity_message = "正在检测外网".into();
    let report: ProbeReport = connectivity::probe(interface, &cancel);
    cancel.check()?;
    slot.connectivity = report.connectivity;
    slot.connectivity_last_run_at = Some(now);
    slot.connectivity_next_run_at = Some(now + config.interval);
    slot.connectivity_phase = if report.available() {
        "success"
    } else {
        "failure"
    }
    .into();
    slot.connectivity_message = format!("外网状态：{}", report.connectivity.label());
    if matches!(test, Some(TestKind::Connectivity)) {
        return Ok((
            if report.available() {
                "online"
            } else {
                "retry"
            },
            slot.connectivity_message.clone(),
            config.interval,
        ));
    }
    let campus = slot
        .session
        .campus_ip()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("interface:{}:{}", interface.name, interface.address));
    let exit = book.exit(config.gateway_ip, &campus);
    if report.available() && manual.is_none() && test.is_none() {
        exit.healthy();
        if slot.browser_enabled {
            slot.browser_next_run_at = None;
            slot.browser_phase = "waiting".into();
            slot.browser_message = "等待外网异常".into();
        }
        return Ok((
            "online",
            format!("网络正常 · {}", interface.name),
            config.interval,
        ));
    }
    if let Some(reason) = &exit.blocked {
        return Ok(("needs_user", reason.clone(), config.interval));
    }
    if exit.cooling(Instant::now()) {
        if report.available() {
            exit.healthy();
            return Ok((
                "online",
                "网络正常；修复请求处于冷却期，未重复注销".into(),
                config.interval,
            ));
        }
        let remaining = exit
            .cooldown_until
            .unwrap()
            .saturating_duration_since(Instant::now())
            .as_secs()
            .max(1);
        slot.retry_at = Some(unix_now() + remaining);
        if slot.browser_enabled {
            slot.browser_next_run_at = slot.retry_at;
            slot.browser_phase = "cooldown".into();
            slot.browser_message = "等待恢复冷却结束".into();
        }
        return Ok((
            "cooldown",
            "恢复冷却中，继续检测外网，暂不重复注销或启动浏览器".into(),
            config.interval.min(remaining),
        ));
    }
    if manual == Some(RecoveryMode::Browser) && !config.browser_fallback
        || matches!(test, Some(TestKind::Browser)) && !cfg!(feature = "browser-fallback")
    {
        return Ok((
            "retry",
            "网页兜底未启用或当前构建不支持".into(),
            config.interval,
        ));
    }
    if !exit.failed(Instant::now()) && manual.is_none() {
        return Ok((
            "suspect",
            format!(
                "{}，正在复核外网：{}",
                if slot.authentication == Authentication::Online {
                    "已认证但外网异常"
                } else {
                    "连接异常"
                },
                report.summary()
            ),
            recovery::CONFIRM_INTERVAL,
        ));
    }
    if manual.is_none() && test.is_none() && !config.portal_auth_enabled && !config.browser_fallback
    {
        return Ok((
            "retry",
            "外网不可用；自动修复已关闭".into(),
            config.interval,
        ));
    }
    if !exit.begin(Instant::now()) {
        return Ok(("cooldown", "恢复冷却中".into(), config.interval));
    }
    let mode = if matches!(test, Some(TestKind::Browser)) {
        RecoveryMode::Browser
    } else {
        manual.unwrap_or(if config.portal_auth_enabled {
            RecoveryMode::Protocol
        } else {
            RecoveryMode::Browser
        })
    };
    if mode == RecoveryMode::Browser || config.browser_fallback {
        slot.browser_phase = "running".into();
        slot.browser_message = "正在运行网页兜底".into();
        slot.browser_next_run_at = None;
    }
    let snapshot = DashboardSnapshot {
        phase: String::new(),
        interface: Some(interface.name.clone()),
        local_ip: Some(interface.address),
        campus_ip: slot.session.campus_ip().map(str::to_owned),
        last_check_at: Some(unix_now()),
        retry_at: None,
        interval: config.interval,
        authentication: slot.authentication,
        connectivity: slot.connectivity,
        recovery_stage: None,
        tasks: task_snapshots(slot),
    };
    let progress = |phase: &str, message: &str| {
        if cancel.check().is_err() {
            return;
        }
        let mut event = Event::new("dashboard", message);
        let mut snapshot = snapshot.clone();
        snapshot.phase = phase.into();
        snapshot.recovery_stage = Some(phase.into());
        event.dashboard = Some(snapshot);
        emit_safe(emit, config, event);
        emit_safe(emit, config, Event::new(phase, message));
    };
    let mut ops = SessionRecovery {
        session: &mut slot.session,
        config,
        cancel: cancel.clone(),
        authentication: &mut slot.authentication,
        connectivity: &mut slot.connectivity,
    };
    let result = recovery::execute(
        &mut ops,
        mode,
        (config.browser_fallback || matches!(test, Some(TestKind::Browser)))
            && cfg!(feature = "browser-fallback"),
        &progress,
    );
    cancel.check()?;
    match result {
        Ok(()) => {
            exit.healthy();
            let completed = unix_now();
            slot.connectivity_last_run_at = Some(completed);
            slot.connectivity_next_run_at = Some(completed + config.interval);
            slot.connectivity_phase = "success".into();
            slot.connectivity_message = "外网可用".into();
            if mode == RecoveryMode::Browser {
                slot.browser_last_run_at = Some(completed);
                slot.browser_phase = "success".into();
                slot.browser_message = "网页兜底验证成功".into();
            }
            Ok(("online", "恢复成功，外网已验证可用".into(), config.interval))
        }
        Err(error) if error.kind == AuthErrorKind::NeedsUser => {
            exit.blocked = Some(error.to_string());
            Ok(("needs_user", error.to_string(), config.interval))
        }
        Err(error) if error.kind == AuthErrorKind::Cancelled => Err(error),
        Err(error) => {
            let remaining = exit
                .cooldown_until
                .unwrap()
                .saturating_duration_since(Instant::now())
                .as_secs()
                .max(1);
            slot.retry_at = Some(unix_now() + remaining);
            slot.browser_next_run_at = Some(unix_now() + remaining);
            slot.browser_phase = "cooldown".into();
            slot.browser_message = error.to_string();
            Ok((
                "cooldown",
                format!("{}；进入恢复冷却，继续检测外网", error),
                config.interval.min(remaining),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retry_sequence() {
        assert_eq!(
            (1..=8).map(retry_delay).collect::<Vec<_>>(),
            [30, 60, 120, 240, 300, 300, 300, 300]
        );
    }
    #[test]
    fn stop_is_immediate_without_requests() {
        let config = Config {
            username: "test".into(),
            password: "test".to_string().into(),
            interface: Some("nonexistent-test-interface".into()),
            ..Config::default()
        };
        let mut handle = Handle::spawn(config, Mode::Monitor, |_| {}).unwrap();
        let now = Instant::now();
        handle.signal.stop();
        handle.join();
        assert!(now.elapsed() < Duration::from_secs(1));
    }
    #[test]
    fn task_schedule_reasons_distinguish_disabled_scheduled_and_triggered() {
        assert_eq!(
            task_snapshot(false, "idle", "", None, None, false).schedule_reason,
            TaskScheduleReason::Disabled
        );
        assert_eq!(
            task_snapshot(true, "idle", "", None, None, true).schedule_reason,
            TaskScheduleReason::WaitingTrigger
        );
        assert_eq!(
            task_snapshot(true, "success", "", None, Some(unix_now() + 10), false).schedule_reason,
            TaskScheduleReason::Scheduled
        );
        assert_eq!(
            task_snapshot(true, "cooldown", "", None, Some(unix_now() + 10), true).schedule_reason,
            TaskScheduleReason::Cooldown
        );
    }
}
