use crate::{
    interfaces::{self, Interface, InterfaceRecord},
    transport::{Portal, Session},
    Config, DashboardSnapshot, Event,
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

#[derive(Clone, Copy)]
pub enum Mode {
    Monitor,
    Inspect,
    Status,
    Once,
}
#[derive(Clone)]
enum Command {
    Stop,
    Network(Vec<String>),
    Suspend,
    Resume,
    Tick,
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
        config.validate(matches!(mode, Mode::Monitor | Mode::Once))?;
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
    let mut event = Event::new("dashboard", message);
    event.dashboard = Some(DashboardSnapshot {
        phase: phase.into(),
        interface: current.map(|(i, _)| i.name.clone()),
        local_ip: current.map(|(i, _)| i.address),
        campus_ip: current.and_then(|(_, s)| s.session.campus_ip().map(str::to_owned)),
        last_check_at: current.and_then(|(_, s)| s.last_check_at),
        retry_at: if phase == "retry" {
            current.and_then(|(_, s)| s.retry_at)
        } else {
            None
        },
        interval,
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
                Mode::Status => session.inspect().and_then(|_| session.online()),
                Mode::Once => session
                    .login(&config.username, &config.password)
                    .map(|_| true),
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
            if !matches!(mode, Mode::Inspect) {
                emit_safe(
                    emit,
                    &config,
                    Event::new(
                        if campus { "online" } else { "error" },
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
    loop {
        // Drain controls before starting any new request.
        while let Ok(command) = rx.try_recv() {
            if apply_control(command, &mut controls, &mut slots, config.interval, emit) {
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
                let result = slot.session.login(&config.username, &config.password);
                if generation.load(Ordering::SeqCst) != active {
                    break;
                }
                slot.last_check_at = Some(unix_now());
                let event = match result {
                    Ok(_) => {
                        slot.failures = 0;
                        slot.retry_at = None;
                        slot.online = true;
                        slot.due = Instant::now() + Duration::from_secs(config.interval);
                        preferred = Some(interface.clone());
                        Event::new(
                            "online",
                            format!(
                                "校园网在线 · {} · 本地 {} · 校园网 {}",
                                interface.name,
                                interface.address,
                                slot.session.campus_ip().unwrap_or("未知")
                            ),
                        )
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
                        event.retry_at = slot.retry_at;
                        event
                    }
                };
                slot.message = if slot.online {
                    "校园网已认证，自动重连正在运行。".into()
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
                if apply_control(c, &mut controls, &mut slots, config.interval, emit) {
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
    interval: u64,
    emit: &dyn Fn(Event),
) -> bool {
    match command {
        Command::Stop => return true,
        Command::Suspend => {
            emit_dashboard(emit, interval, "suspended", "睡眠期间暂停", None);
            controls.suspend();
            slots.clear();
        }
        Command::Network(names) => controls.network(Instant::now(), names),
        Command::Resume => controls.resume(Instant::now()),
        Command::Tick => {}
    }
    false
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
}
