use buaa_core::{
    engine::{Handle, Mode, TestKind},
    Config, Event,
};
use serde::Serialize;
use std::{
    collections::HashMap,
    net::Ipv4Addr,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{broadcast, oneshot};

use crate::storage::{timestamp, Account, Settings, Storage};

#[derive(Clone, Serialize)]
pub struct AccountSummary {
    pub username: String,
    pub has_password: bool,
    pub remembered: bool,
}

#[derive(Clone, Serialize)]
pub struct UiInterface {
    pub name: String,
    pub binding: String,
    pub label: String,
    pub message: String,
    pub campus: bool,
    pub campus_ip: Option<String>,
    pub kind: String,
}

#[derive(Clone, Serialize)]
pub struct Activity {
    pub at: u64,
    pub message: String,
}

#[derive(Clone, Serialize)]
pub struct UiState {
    pub revision: u64,
    pub phase: String,
    pub message: String,
    pub monitoring: bool,
    pub busy: bool,
    pub testing: Option<String>,
    pub account: AccountSummary,
    pub settings: Settings,
    pub interfaces: Vec<UiInterface>,
    pub dashboard: Option<serde_json::Value>,
    pub activities: Vec<Activity>,
}

pub struct Model {
    pub state: UiState,
    pub account: Option<Account>,
}

impl Model {
    pub fn new(settings: Settings, account: Option<Account>) -> Self {
        let account_summary = summary(account.as_ref());
        Self {
            state: UiState {
                revision: 1,
                phase: "idle".into(),
                message: "请设置账号并检测校园网接口。".into(),
                monitoring: false,
                busy: false,
                testing: None,
                account: account_summary,
                settings,
                interfaces: vec![],
                dashboard: None,
                activities: vec![],
            },
            account,
        }
    }

    pub fn refresh_account_summary(&mut self) {
        self.state.account = summary(self.account.as_ref());
    }

    pub fn config(&self, credentials: bool) -> Result<Config, String> {
        self.state.settings.validate()?;
        let account = self.account.as_ref();
        let config = Config {
            username: account
                .map(|value| value.username.trim().to_owned())
                .unwrap_or_default(),
            password: account
                .map(|value| value.password.to_string())
                .unwrap_or_default()
                .into(),
            interfaces: self.state.settings.interfaces.clone(),
            gateway_ip: self
                .state
                .settings
                .gateway_ip
                .parse()
                .map_err(|_| "默认网关必须是 IPv4 地址")?,
            interface_gateways: self
                .state
                .settings
                .interface_gateways
                .iter()
                .map(|(key, value)| {
                    value
                        .parse::<Ipv4Addr>()
                        .map(|value| (key.clone(), value))
                        .map_err(|_| "接口网关必须是 IPv4 地址".to_string())
                })
                .collect::<Result<HashMap<_, _>, _>>()?,
            interval: self.state.settings.interval,
            try_all: false,
            portal_auth_enabled: self.state.settings.portal_auth_enabled,
            connectivity_probe_enabled: self.state.settings.connectivity_probe_enabled,
            browser_fallback: false,
            ..Config::default()
        };
        config.validate(credentials)?;
        Ok(config)
    }
}

fn summary(account: Option<&Account>) -> AccountSummary {
    AccountSummary {
        username: account
            .map(|value| value.username.clone())
            .unwrap_or_default(),
        has_password: account.is_some_and(|value| !value.password.is_empty()),
        remembered: account.is_some_and(|value| value.remembered),
    }
}

pub enum Command {
    Start(oneshot::Sender<Result<(), String>>),
    Stop(oneshot::Sender<Result<(), String>>),
    Inspect(oneshot::Sender<Result<(), String>>),
    Test(TestKind, oneshot::Sender<Result<(), String>>),
    SetInterval(u64, oneshot::Sender<Result<(), String>>),
    Restart(u64),
    Core(u64, Box<Event>),
    Shutdown(oneshot::Sender<()>),
}

#[derive(Clone)]
pub struct Supervisor {
    tx: crossbeam_channel::Sender<Command>,
}

impl Supervisor {
    pub fn start(
        storage: Storage,
        model: Arc<Mutex<Model>>,
        events: broadcast::Sender<String>,
    ) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let actor_tx = tx.clone();
        std::thread::Builder::new()
            .name("web-supervisor".into())
            .spawn(move || run(storage, model, events, actor_tx, rx))
            .expect("unable to start supervisor");
        Self { tx }
    }

    pub async fn start_monitor(&self) -> Result<(), String> {
        self.request(Command::Start).await
    }

    pub async fn stop_monitor(&self) -> Result<(), String> {
        self.request(Command::Stop).await
    }

    pub async fn inspect(&self) -> Result<(), String> {
        self.request(Command::Inspect).await
    }

    pub async fn test(&self, kind: TestKind) -> Result<(), String> {
        self.request(|reply| Command::Test(kind, reply)).await
    }

    pub async fn set_interval(&self, interval: u64) -> Result<(), String> {
        self.request(|reply| Command::SetInterval(interval, reply))
            .await
    }

    pub async fn shutdown(&self) {
        let (tx, rx) = oneshot::channel();
        if self.tx.send(Command::Shutdown(tx)).is_ok() {
            let _ = tokio::time::timeout(Duration::from_secs(6), rx).await;
        }
    }

    async fn request(
        &self,
        command: impl FnOnce(oneshot::Sender<Result<(), String>>) -> Command,
    ) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(command(tx))
            .map_err(|_| "服务控制器已停止".to_string())?;
        rx.await.map_err(|_| "服务控制器未响应".to_string())?
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ActiveMode {
    Monitor,
    Inspect,
    Test,
}

fn run(
    storage: Storage,
    model: Arc<Mutex<Model>>,
    events: broadcast::Sender<String>,
    tx: crossbeam_channel::Sender<Command>,
    rx: crossbeam_channel::Receiver<Command>,
) {
    let mut handle: Option<Handle> = None;
    let mut mode: Option<ActiveMode> = None;
    let mut epoch = 0u64;
    let mut shutting_down = false;
    let mut shutdown_reply: Option<oneshot::Sender<()>> = None;
    while let Ok(command) = rx.recv() {
        match command {
            Command::Start(reply) => {
                let result = if handle.is_some() {
                    Err("当前已有任务正在运行".into())
                } else {
                    let config = model.lock().unwrap().config(true);
                    match config.and_then(|config| spawn(config, Mode::Monitor, epoch + 1, &tx)) {
                        Ok(next) => {
                            epoch += 1;
                            handle = Some(next);
                            mode = Some(ActiveMode::Monitor);
                            let mut data = model.lock().unwrap();
                            data.state.settings.monitor_enabled = true;
                            data.state.monitoring = true;
                            data.state.busy = false;
                            data.state.phase = "checking".into();
                            data.state.message = "正在检查校园网连接…".into();
                            let save = storage.save_settings(&data.state.settings);
                            notify(&mut data, &events);
                            save
                        }
                        Err(error) => Err(error),
                    }
                };
                let _ = reply.send(result);
            }
            Command::Stop(reply) => {
                {
                    let mut data = model.lock().unwrap();
                    data.state.settings.monitor_enabled = false;
                    data.state.busy = handle.is_some();
                    data.state.message = if handle.is_some() {
                        "正在停止…".into()
                    } else {
                        "自动连接已停止".into()
                    };
                    let _ = storage.save_settings(&data.state.settings);
                    notify(&mut data, &events);
                }
                if let Some(handle) = &handle {
                    handle.signal.stop();
                }
                let _ = reply.send(Ok(()));
            }
            Command::Inspect(reply) => {
                let result = if handle.is_some() {
                    Err("请先停止当前任务再检测接口".into())
                } else {
                    let config = model.lock().unwrap().config(false).map(|mut config| {
                        // Inspection always enumerates all host-visible interfaces. Saved
                        // selections only constrain monitor and test runs.
                        config.interfaces.clear();
                        config.interface = None;
                        config.try_all = true;
                        config
                    });
                    match config.and_then(|config| spawn(config, Mode::Inspect, epoch + 1, &tx)) {
                        Ok(next) => {
                            epoch += 1;
                            handle = Some(next);
                            mode = Some(ActiveMode::Inspect);
                            let mut data = model.lock().unwrap();
                            data.state.busy = true;
                            data.state.phase = "checking".into();
                            data.state.message = "正在检测校园网接口…".into();
                            notify(&mut data, &events);
                            Ok(())
                        }
                        Err(error) => Err(error),
                    }
                };
                let _ = reply.send(result);
            }
            Command::Test(kind, reply) => {
                let testing = model.lock().unwrap().state.testing.is_some();
                let result = if testing {
                    Err("已有测试正在运行".into())
                } else if mode == Some(ActiveMode::Monitor) {
                    if let Some(handle) = &handle {
                        handle.signal.test(kind);
                    }
                    let mut data = model.lock().unwrap();
                    data.state.testing = Some(test_name(kind).into());
                    notify(&mut data, &events);
                    Ok(())
                } else if handle.is_some() {
                    Err("当前已有任务正在运行".into())
                } else {
                    let config = model.lock().unwrap().config(kind == TestKind::PortalAuth);
                    match config.and_then(|config| spawn(config, Mode::Test(kind), epoch + 1, &tx))
                    {
                        Ok(next) => {
                            epoch += 1;
                            handle = Some(next);
                            mode = Some(ActiveMode::Test);
                            let mut data = model.lock().unwrap();
                            data.state.busy = true;
                            data.state.testing = Some(test_name(kind).into());
                            notify(&mut data, &events);
                            Ok(())
                        }
                        Err(error) => Err(error),
                    }
                };
                let _ = reply.send(result);
            }
            Command::SetInterval(value, reply) => {
                let result = if !(10..=86400).contains(&value) {
                    Err("检查周期须为 10–86400 秒".into())
                } else {
                    (|| -> Result<(), String> {
                        if mode == Some(ActiveMode::Monitor) {
                            if let Some(handle) = &handle {
                                handle.signal.set_interval(value)?;
                            }
                        }
                        let mut data = model.lock().unwrap();
                        data.state.settings.interval = value;
                        storage.save_settings(&data.state.settings)?;
                        notify(&mut data, &events);
                        Ok(())
                    })()
                };
                let _ = reply.send(result);
            }
            Command::Core(event_epoch, event) if event_epoch == epoch => {
                let stopped = event.state == "stopped";
                let current_mode = mode;
                {
                    let mut data = model.lock().unwrap();
                    apply_event(&storage, &mut data, &event);
                    notify(&mut data, &events);
                }
                if stopped {
                    if let Some(mut current) = handle.take() {
                        current.join();
                    }
                    mode = None;
                    let desired = model.lock().unwrap().state.settings.monitor_enabled;
                    if shutting_down {
                        if let Some(reply) = shutdown_reply.take() {
                            let _ = reply.send(());
                        }
                        break;
                    }
                    if current_mode == Some(ActiveMode::Monitor) && desired {
                        let restart_tx = tx.clone();
                        let restart_epoch = epoch;
                        std::thread::spawn(move || {
                            std::thread::sleep(Duration::from_secs(30));
                            let _ = restart_tx.send(Command::Restart(restart_epoch));
                        });
                    }
                }
            }
            Command::Core(_, _) => {}
            Command::Restart(restart_epoch) if restart_epoch == epoch && handle.is_none() => {
                if model.lock().unwrap().state.settings.monitor_enabled {
                    let (reply, _) = oneshot::channel();
                    let _ = tx.send(Command::Start(reply));
                }
            }
            Command::Restart(_) => {}
            Command::Shutdown(reply) => {
                shutting_down = true;
                if let Some(handle) = &handle {
                    shutdown_reply = Some(reply);
                    handle.signal.stop();
                } else {
                    let _ = reply.send(());
                    break;
                }
            }
        }
    }
}

fn spawn(
    config: Config,
    mode: Mode,
    epoch: u64,
    tx: &crossbeam_channel::Sender<Command>,
) -> Result<Handle, String> {
    let tx = tx.clone();
    Handle::spawn(config, mode, move |event| {
        let _ = tx.send(Command::Core(epoch, Box::new(event)));
    })
}

fn apply_event(storage: &Storage, data: &mut Model, event: &Event) {
    if let Some(snapshot) = &event.dashboard {
        data.state.phase = snapshot.phase.clone();
        data.state.dashboard = serde_json::to_value(snapshot).ok();
    }
    if event.state == "interfaces" {
        data.state.interfaces = event
            .records
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|record| UiInterface {
                name: record.interface.name.clone(),
                binding: record.binding.clone(),
                label: record.label.clone(),
                message: record.message.clone(),
                campus: record.campus,
                campus_ip: record.campus_ip.clone(),
                kind: interface_kind(&record.interface.name),
            })
            .collect();
    }
    if event.state == "test_result" {
        data.state.testing = None;
    }
    if event.state == "stopped" {
        data.state.monitoring = false;
        data.state.busy = false;
        data.state.testing = None;
        data.state.phase = "stopped".into();
    }
    if !event.message.is_empty() {
        data.state.message = event.message.clone();
    }
    if !matches!(event.state.as_str(), "schedule" | "dashboard" | "checking")
        && !event.message.is_empty()
    {
        record(storage, data, &event.message);
    }
}

pub fn record(storage: &Storage, data: &mut Model, message: &str) {
    let message = redact(data, message);
    let at = timestamp();
    data.state.activities.push(Activity {
        at,
        message: message.clone(),
    });
    if data.state.activities.len() > 120 {
        let remove = data.state.activities.len() - 120;
        data.state.activities.drain(0..remove);
    }
    let _ = storage.append_log(&format!("{at}  {message}"));
}

fn redact(data: &Model, message: &str) -> String {
    let mut output = message.to_owned();
    if let Some(account) = &data.account {
        if !account.username.is_empty() {
            output = output.replace(&account.username, "[账号]");
        }
        if !account.password.is_empty() {
            output = output.replace(account.password.as_str(), "[密码]");
        }
    }
    output
}

pub fn notify(data: &mut Model, events: &broadcast::Sender<String>) {
    data.state.revision = data.state.revision.wrapping_add(1);
    if let Ok(json) = serde_json::to_string(&data.state) {
        let _ = events.send(json);
    }
}

fn interface_kind(name: &str) -> String {
    if Path::new("/sys/class/net")
        .join(name)
        .join("wireless")
        .exists()
    {
        "wifi".into()
    } else {
        "wired".into()
    }
}

fn test_name(kind: TestKind) -> &'static str {
    match kind {
        TestKind::PortalAuth => "portal",
        TestKind::Connectivity => "connectivity",
        TestKind::Browser => "browser",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeroize::Zeroizing;

    #[test]
    fn automatic_and_multi_interface_configuration() {
        let mut settings = Settings::default();
        settings.portal_auth_enabled = false;
        let model = Model::new(settings.clone(), None);
        let config = model.config(false).unwrap();
        assert!(config.interfaces.is_empty());
        assert!(!config.try_all);

        settings.interfaces = vec!["eth0".into(), "wlan0".into()];
        settings
            .interface_gateways
            .insert("wlan0".into(), "10.0.0.1".into());
        let config = Model::new(settings, None).config(false).unwrap();
        assert_eq!(config.interfaces.len(), 2);
        assert_eq!(config.interface_gateways.len(), 1);
        assert!(config.monitors_all_selected());
    }

    #[test]
    fn activity_redacts_account_and_password() {
        let mut model = Model::new(
            Settings::default(),
            Some(Account {
                username: "student-id".into(),
                password: Zeroizing::new("very-secret".into()),
                remembered: true,
            }),
        );
        let message = redact(&model, "student-id failed with very-secret");
        assert_eq!(message, "[账号] failed with [密码]");
        model.account = None;
        assert_eq!(redact(&model, "safe"), "safe");
    }

    #[tokio::test]
    async fn rejects_overlapping_tests() {
        let path = std::env::temp_dir().join(format!(
            "buaalogin-supervisor-test-{}-{}",
            std::process::id(),
            timestamp()
        ));
        let storage = Storage::new(path).unwrap();
        let model = Arc::new(Mutex::new(Model::new(Settings::default(), None)));
        model.lock().unwrap().state.testing = Some("portal".into());
        let (events, _) = broadcast::channel(4);
        let supervisor = Supervisor::start(storage, model, events);
        assert_eq!(
            supervisor.test(TestKind::Connectivity).await.unwrap_err(),
            "已有测试正在运行"
        );
        supervisor.shutdown().await;
    }
}
