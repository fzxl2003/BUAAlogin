mod app;
mod auth;
mod credentials;
mod storage;
mod supervisor;

use std::{
    env,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::broadcast;
use zeroize::Zeroizing;

use app::AppState;
use auth::AuthState;
use storage::{Account, Settings, Storage};
use supervisor::{record, Model, Supervisor};

#[tokio::main]
async fn main() {
    if env::args().nth(1).as_deref() == Some("healthcheck") {
        std::process::exit(if healthcheck() { 0 } else { 1 });
    }
    if let Err(error) = run().await {
        eprintln!("错误：{error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let bind = env::var("WEB_BIND").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let address: SocketAddr = bind.parse().map_err(|_| "WEB_BIND 必须是 IP:端口")?;
    let root = PathBuf::from(env::var_os("BUAA_DATA_DIR").unwrap_or_else(|| "/data".into()));
    let storage = Storage::new(root)?;
    storage.cleanup_logs();
    let fresh = !storage.settings_exist();
    let (mut settings, mut account) = if fresh {
        import_legacy()
    } else {
        (storage.load_settings()?, storage.load_account()?)
    };
    if fresh {
        storage.save_settings(&settings)?;
        if let Some(account) = &account {
            storage.save_account(account)?;
        }
    }
    if settings.monitor_enabled && account.is_none() {
        settings.monitor_enabled = false;
        storage.save_settings(&settings)?;
    }
    let password_hash = storage.read_auth_hash()?;
    let auth = AuthState::new(password_hash);
    if let Some(token) = auth.setup_token() {
        println!("BUAALogin 网页端首次设置码：{token}");
        println!(
            "请打开 http://<服务器地址>:{} 并输入该设置码；设置前控制 API 保持关闭。",
            address.port()
        );
    }
    let auth = Arc::new(Mutex::new(auth));
    let model = Arc::new(Mutex::new(Model::new(settings.clone(), account.take())));
    let (events, _) = broadcast::channel(64);
    let supervisor = Supervisor::start(storage.clone(), model.clone(), events.clone());
    if settings.monitor_enabled {
        if let Err(error) = supervisor.start_monitor().await {
            let mut data = model.lock().unwrap();
            data.state.settings.monitor_enabled = false;
            record(&storage, &mut data, &format!("自动恢复监控失败：{error}"));
            let _ = storage.save_settings(&data.state.settings);
        }
    }
    let state = AppState {
        storage,
        model,
        auth,
        supervisor: supervisor.clone(),
        events,
    };
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|error| format!("无法监听 {address}：{error}"))?;
    println!("BUAALogin 网页端正在监听 http://{address}");
    axum::serve(listener, app::router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|error| format!("网页服务异常：{error}"))?;
    supervisor.shutdown().await;
    Ok(())
}

fn import_legacy() -> (Settings, Option<Account>) {
    import_legacy_from(|name| env::var(name).ok())
}

fn import_legacy_from(get: impl Fn(&str) -> Option<String>) -> (Settings, Option<Account>) {
    let mut settings = Settings::default();
    if let Some(interface) = get("INTERFACE") {
        if !interface.trim().is_empty() {
            settings.interfaces = vec![interface.trim().into()];
        }
    }
    let username = get("USERNAME").unwrap_or_default();
    let password = get("PASSWORD").unwrap_or_default();
    let account = if !username.trim().is_empty() && !password.is_empty() {
        settings.monitor_enabled = true;
        Some(Account {
            username: username.trim().into(),
            password: Zeroizing::new(password),
            remembered: true,
        })
    } else {
        None
    };
    (settings, account)
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut terminate = signal(SignalKind::terminate()).expect("SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = terminate.recv() => {},
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}

fn healthcheck() -> bool {
    let bind = env::var("WEB_BIND").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let Ok(mut address) = bind.parse::<SocketAddr>() else {
        return false;
    };
    if address.ip().is_unspecified() {
        address.set_ip("127.0.0.1".parse().unwrap());
    }
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_secs(2)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    if stream
        .write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut response = [0u8; 64];
    let Ok(read) = stream.read(&mut response) else {
        return false;
    };
    response[..read].starts_with(b"HTTP/1.1 200")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn imports_legacy_environment_only_when_complete() {
        let values = HashMap::from([
            ("USERNAME", " 20260001 ".to_string()),
            ("PASSWORD", "secret".to_string()),
            ("INTERFACE", " eth0 ".to_string()),
        ]);
        let (settings, account) = import_legacy_from(|key| values.get(key).cloned());
        assert!(settings.monitor_enabled);
        assert_eq!(settings.interfaces, ["eth0"]);
        let account = account.unwrap();
        assert_eq!(account.username, "20260001");
        assert_eq!(account.password.as_str(), "secret");

        let (settings, account) =
            import_legacy_from(|key| (key == "USERNAME").then(|| "20260001".to_string()));
        assert!(!settings.monitor_enabled);
        assert!(account.is_none());
    }
}
