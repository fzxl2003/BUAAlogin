use buaa_core::{
    engine::{Handle, Mode},
    Config,
};
use std::{
    env,
    process::ExitCode,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("错误：{e}");
            ExitCode::from(1)
        }
    }
}
fn run() -> Result<u8, String> {
    let mut config = Config {
        username: env::var("USERNAME").unwrap_or_default().trim().into(),
        password: env::var("PASSWORD").unwrap_or_default().into(),
        interface: env::var("INTERFACE").ok().filter(|s| !s.is_empty()),
        try_all: true,
        ..Config::default()
    };
    let mut mode = Mode::Monitor;
    let mut debug = false;
    let mut log_file = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!("BUAALogin\n环境变量：USERNAME PASSWORD INTERFACE\n参数：--status --once --interface NAME --gateway-ip IPV4 --interval SECONDS --debug --log-file PATH");
                return Ok(0);
            }
            "--status" => mode = Mode::Status,
            "--once" => mode = Mode::Once,
            "--interface" => config.interface = Some(args.next().ok_or("缺少接口名")?),
            "--gateway-ip" => {
                config.gateway_ip = args
                    .next()
                    .ok_or("缺少网关地址")?
                    .parse()
                    .map_err(|_| "网关必须是 IPv4")?
            }
            "--interval" => {
                config.interval = args
                    .next()
                    .ok_or("缺少间隔")?
                    .parse()
                    .map_err(|_| "检查间隔必须是整数")?
            }
            "--debug" => debug = true,
            "--log-file" => {
                log_file = Some(args.next().ok_or("缺少日志路径")?);
                debug = true;
            }
            _ => return Err(format!("未知参数：{arg}")),
        }
    }
    // Docker/noninteractive commands require credentials even for --status.
    config.validate(true)?;
    let mut file = log_file
        .map(|p| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
        })
        .transpose()
        .map_err(|_| "无法打开日志文件")?;
    let success = Arc::new(AtomicBool::new(matches!(mode, Mode::Monitor)));
    let completed = success.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut handle = Handle::spawn(config, mode, move |event| {
        if event.state == "failed" {
            completed.store(false, Ordering::SeqCst);
        }
        if event.state == "complete" {
            completed.store(true, Ordering::SeqCst);
        }
        let _ = tx.send(event);
    })?;
    let signal = handle.signal.clone();
    #[cfg(unix)]
    let mut signals = signal_hook::iterator::Signals::new([
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGINT,
    ])
    .map_err(|_| "无法注册退出信号")?;
    #[cfg(unix)]
    let signal_handle = signals.handle();
    #[cfg(unix)]
    let signal_thread = std::thread::spawn(move || {
        if signals.forever().next().is_some() {
            signal.stop();
        }
    });
    #[cfg(windows)]
    register_console_stop(signal)?;
    for event in rx {
        if event.state != "schedule"
            && event.state != "dashboard"
            && (debug || event.state != "checking")
        {
            let line = serde_json::to_string(&event).unwrap();
            println!("{line}");
            if let Some(file) = file.as_mut() {
                use std::io::Write;
                let _ = writeln!(file, "{line}");
            }
        }
        if event.state == "stopped" {
            break;
        }
    }
    handle.join();
    #[cfg(unix)]
    {
        signal_handle.close();
        let _ = signal_thread.join();
    }
    Ok(if success.load(Ordering::SeqCst) { 0 } else { 1 })
}

#[cfg(windows)]
fn register_console_stop(signal: buaa_core::engine::Signal) -> Result<(), String> {
    use windows_sys::Win32::System::Console::*;
    static SIGNAL: std::sync::OnceLock<buaa_core::engine::Signal> = std::sync::OnceLock::new();
    unsafe extern "system" fn stop(kind: u32) -> i32 {
        if [
            CTRL_C_EVENT,
            CTRL_BREAK_EVENT,
            CTRL_CLOSE_EVENT,
            CTRL_SHUTDOWN_EVENT,
        ]
        .contains(&kind)
        {
            if let Some(signal) = SIGNAL.get() {
                signal.stop();
            }
            return 1;
        }
        0
    }
    let _ = SIGNAL.set(signal);
    if unsafe { SetConsoleCtrlHandler(Some(stop), 1) } == 0 {
        return Err("无法注册退出通知".into());
    }
    Ok(())
}
