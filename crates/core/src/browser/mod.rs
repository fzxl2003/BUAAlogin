mod cdp;
mod proxy;
use crate::{cancellation::Cancellation, interfaces::Interface, transport::AuthError, Config};
use cdp::Cdp;
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

pub(crate) fn recover(
    interface: &Interface,
    config: &Config,
    cancel: &Cancellation,
    progress: &dyn Fn(&str),
) -> Result<(), AuthError> {
    let cancel = cancel.with_timeout(Duration::from_secs(90));
    cancel.check()?;
    let executable = find_browser(config.browser_path.as_deref())?;
    let proxy = proxy::Proxy::start(
        interface.clone(),
        config.gateway_for(interface),
        cancel.clone(),
    )?;
    cancel.check()?;
    let mut browser = Browser::launch(&executable, Some(proxy.address))?;
    let mut cdp = browser.connect(&cancel, "https://gw.buaa.edu.cn")?;
    drive(
        &mut cdp,
        &cancel,
        "https://gw.buaa.edu.cn/",
        &config.username,
        &config.password,
        progress,
    )
}

fn find_browser(configured: Option<&str>) -> Result<PathBuf, AuthError> {
    if let Some(path) = configured.filter(|path| !path.trim().is_empty()) {
        return Path::new(path)
            .is_file()
            .then(|| PathBuf::from(path))
            .ok_or_else(|| "网页兜底不可用：指定浏览器不存在".into());
    }
    let mut candidates = Vec::new();
    #[cfg(target_os = "macos")]
    {
        for root in [
            Some(PathBuf::from("/Applications")),
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Applications")),
        ]
        .into_iter()
        .flatten()
        {
            for (bundle, executable) in [
                ("Google Chrome", "Google Chrome"),
                ("Microsoft Edge", "Microsoft Edge"),
                ("Chromium", "Chromium"),
            ] {
                candidates.push(root.join(format!("{bundle}.app/Contents/MacOS/{executable}")));
            }
        }
    }
    #[cfg(windows)]
    for relative in [
        "Microsoft/Edge/Application/msedge.exe",
        "Google/Chrome/Application/chrome.exe",
        "Chromium/Application/chrome.exe",
    ] {
        for variable in ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"] {
            if let Some(root) = std::env::var_os(variable) {
                candidates.push(PathBuf::from(root).join(relative));
            }
        }
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    for path in [
        "/usr/bin/google-chrome",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
    ] {
        candidates.push(PathBuf::from(path));
    }
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| "网页兜底不可用：请安装 Chrome、Edge 或 Chromium，或指定浏览器路径".into())
}

struct Browser {
    child: Child,
    profile: tempfile::TempDir,
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
}
impl Browser {
    fn launch(executable: &Path, proxy: Option<SocketAddr>) -> Result<Self, AuthError> {
        let profile = tempfile::Builder::new()
            .prefix("buaa-browser-")
            .tempdir()
            .map_err(|_| "无法创建临时浏览器目录")?;
        std::fs::write(profile.path().join("First Run"), b"").map_err(|_| "无法配置临时浏览器")?;
        std::fs::create_dir(profile.path().join("Default")).map_err(|_| "无法配置临时浏览器")?;
        std::fs::write(profile.path().join("Default/Preferences"), br#"{"credentials_enable_service":false,"profile":{"password_manager_enabled":false},"autofill":{"profile_enabled":false,"credit_card_enabled":false}}"#).map_err(|_| "无法配置临时浏览器")?;
        let mut command = Command::new(executable);
        command
            .args([
                "--headless=new",
                "--remote-debugging-address=127.0.0.1",
                "--remote-debugging-port=0",
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-background-networking",
                "--disable-component-update",
                "--disable-sync",
                "--disable-extensions",
                "--disable-quic",
                "--disable-default-apps",
                "--disable-breakpad",
                "--disable-session-crashed-bubble",
                "--force-webrtc-ip-handling-policy=disable_non_proxied_udp",
                "--window-size=1024,768",
            ])
            .arg(format!("--user-data-dir={}", profile.path().display()));
        if let Some(proxy) = proxy {
            command
                .arg(format!("--proxy-server=http://{proxy}"))
                .arg("--proxy-bypass-list=<-loopback>")
                // The proxy itself must remain reachable while destination DNS is blocked.
                .arg(format!(
                    "--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE {}",
                    proxy.ip()
                ));
        }
        command
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let child = command
            .spawn()
            .map_err(|_| "网页兜底不可用：浏览器启动失败")?;
        let browser = Self {
            child,
            profile,
            #[cfg(windows)]
            job: std::ptr::null_mut(),
        };
        #[cfg(windows)]
        let mut browser = browser;
        #[cfg(windows)]
        unsafe {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::System::JobObjects::*;
            browser.job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if browser.job.is_null() {
                return Err("无法创建浏览器进程组".into());
            }
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                browser.job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as _,
                std::mem::size_of_val(&limits) as u32,
            ) == 0
                || AssignProcessToJobObject(browser.job, browser.child.as_raw_handle() as _) == 0
            {
                return Err("无法管理后台浏览器进程组".into());
            }
        }
        Ok(browser)
    }
    fn connect(&mut self, cancel: &Cancellation, origin: &str) -> Result<Cdp, AuthError> {
        let startup = cancel.with_timeout(Duration::from_secs(15));
        let port = loop {
            startup.check()?;
            if self
                .child
                .try_wait()
                .map_err(|_| "后台浏览器状态读取失败")?
                .is_some()
            {
                return Err("网页兜底不可用：浏览器提前退出".into());
            }
            if let Ok(text) =
                std::fs::read_to_string(self.profile.path().join("DevToolsActivePort"))
            {
                if let Some(port) = text
                    .lines()
                    .next()
                    .and_then(|line| line.parse::<u16>().ok())
                    .filter(|port| *port != 0)
                {
                    break port;
                }
            }
            startup.wait(Duration::from_millis(100))?;
        };
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let targets = read_targets(address, &startup)?;
        let target = targets
            .as_array()
            .and_then(|targets| targets.iter().find(|target| target["type"] == "page"))
            .ok_or("后台浏览器页面未创建")?;
        let url = url::Url::parse(
            target["webSocketDebuggerUrl"]
                .as_str()
                .ok_or("后台浏览器调试地址缺失")?,
        )
        .map_err(|_| "后台浏览器调试地址错误")?;
        if url.scheme() != "ws" || url.host_str() != Some("127.0.0.1") || url.port() != Some(port) {
            return Err("后台浏览器调试地址不被允许".into());
        }
        Cdp::connect(address, url.path(), cancel.clone(), origin.into())
    }
}
impl Drop for Browser {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        #[cfg(windows)]
        unsafe {
            if !self.job.is_null() {
                windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job, 1);
                windows_sys::Win32::Foundation::CloseHandle(self.job);
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn read_targets(address: SocketAddr, cancel: &Cancellation) -> Result<Value, AuthError> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))
        .map_err(|_| "后台浏览器尚未就绪")?;
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .map_err(|_| "后台浏览器连接配置失败")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(1)))
        .map_err(|_| "后台浏览器连接配置失败")?;
    write!(
        stream,
        "GET /json/list HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )
    .map_err(|_| "后台浏览器页面查询失败")?;
    let mut bytes = Vec::new();
    let mut expected = None;
    loop {
        cancel.check()?;
        let mut buffer = [0; 4096];
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) if bytes.len() + count <= 1_048_576 => {
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(position) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    if expected.is_none() {
                        let header = std::str::from_utf8(&bytes[..position])
                            .map_err(|_| "后台浏览器 HTTP 应答错误")?;
                        if !header
                            .lines()
                            .next()
                            .is_some_and(|line| line.starts_with("HTTP/1.1 200 "))
                        {
                            return Err("后台浏览器 HTTP 应答错误".into());
                        }
                        expected = header
                            .lines()
                            .filter_map(|line| line.split_once(':'))
                            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                            .and_then(|(_, length)| length.trim().parse::<usize>().ok());
                    }
                    if expected.is_some_and(|length| bytes.len() >= position + 4 + length) {
                        break;
                    }
                }
            }
            Ok(_) => return Err("后台浏览器应答过大".into()),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(_) => return Err("后台浏览器页面查询失败".into()),
        }
    }
    let body = bytes
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .map(|position| &bytes[position + 4..])
        .ok_or("后台浏览器 HTTP 应答错误")?;
    serde_json::from_slice(body).map_err(|_| "后台浏览器页面应答错误".into())
}
fn drive(
    cdp: &mut Cdp,
    cancel: &Cancellation,
    url: &str,
    username: &str,
    password: &str,
    progress: &dyn Fn(&str),
) -> Result<(), AuthError> {
    cdp.call("Page.enable", json!({}))?;
    cdp.call("Runtime.enable", json!({}))?;
    cdp.call("Page.setDownloadBehavior", json!({"behavior":"deny"}))?;
    let navigation = cdp.call("Page.navigate", json!({"url":url}))?;
    if let Some(error) = navigation.get("errorText") {
        let code = error
            .as_str()
            .filter(|code| {
                code.starts_with("net::ERR_")
                    && code.len() <= 80
                    && code
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b":_".contains(&c))
            })
            .unwrap_or("未知网络错误");
        return Err(format!("认证页面无法加载（{code}），请检查网关、代理与证书").into());
    }
    #[derive(Clone, Copy)]
    enum Phase {
        Detecting,
        LoggingOut,
        AwaitingOnline,
    }
    let mut phase = Phase::Detecting;
    let mut phase_started = Instant::now();
    let mut logout_confirmed = false;
    loop {
        cancel.check()?;
        let page = cdp.evaluate(include_str!("page.js"));
        match page {
            Ok(page) => match page["state"].as_str().unwrap_or("waiting") {
                "needs_user" => {
                    return Err(AuthError::needs_user(&format!(
                        "认证页面需要处理{}：请检查账号、密码、余额或验证码",
                        page_error_code(&page)
                    )))
                }
                "portal_error" => {
                    return Err(format!(
                        "认证页面返回错误{}，请在门户查看详细提示",
                        page_error_code(&page)
                    )
                    .into())
                }
                "ambiguous" => return Err("认证页面控件不唯一，请检查页面适配".into()),
                "wrong_origin" => return Err("认证页面跳转地址不被允许".into()),
                "online" if matches!(phase, Phase::AwaitingOnline) => {
                    progress("网页已确认认证成功");
                    return Ok(());
                }
                "online" if matches!(phase, Phase::Detecting) => {
                    if page["logout_ready"] == false {
                        if phase_started.elapsed() >= Duration::from_secs(30) {
                            return Err("认证页面注销按钮在 30 秒内未就绪，尚未执行注销".into());
                        }
                    } else {
                        progress("检测到注销页面，正在注销当前会话");
                        phase = Phase::LoggingOut;
                        phase_started = Instant::now();
                        cdp.click("logout")?;
                    }
                }
                "login" if !matches!(phase, Phase::AwaitingOnline) => {
                    progress(if matches!(phase, Phase::LoggingOut) {
                        "注销完成，已返回登录页面，正在重新登录"
                    } else {
                        "检测到登录页面，正在填写并提交"
                    });
                    cdp.fill("username", username)?;
                    cdp.fill("password", password)?;
                    cdp.click("login")?;
                    phase = Phase::AwaitingOnline;
                    phase_started = Instant::now();
                    progress("已提交网页登录，正在等待认证结果");
                }
                "confirm_logout" if matches!(phase, Phase::LoggingOut) && !logout_confirmed => {
                    progress("正在确认注销");
                    logout_confirmed = true;
                    cdp.click("confirm")?;
                }
                _ => {}
            },
            Err(error)
                if error.to_string() == "后台浏览器操作失败"
                    || error.to_string() == "认证页面脚本执行失败" => {}
            Err(error) => return Err(error),
        }
        if phase_started.elapsed() >= Duration::from_secs(30) {
            return Err(match phase {
                Phase::Detecting => {
                    "认证页面在 30 秒内未就绪或未识别到登录、注销入口；请检查页面资源加载"
                }
                Phase::LoggingOut => {
                    "网页已请求注销，但 30 秒内未返回登录页面，尚未提交登录；请在门户查看提示"
                }
                Phase::AwaitingOnline => {
                    "网页已提交登录，但 30 秒内未确认认证成功；请在门户查看提示"
                }
            }
            .into());
        }
        cancel.wait(Duration::from_millis(200))?;
    }
}

fn page_error_code(page: &Value) -> String {
    page["code"]
        .as_str()
        .filter(|code| {
            code.len() <= 40 && code.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        })
        .map(|code| format!("（{code}）"))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::RefCell,
        net::TcpListener,
        sync::{
            atomic::{AtomicBool, AtomicU64, Ordering},
            Arc,
        },
    };
    #[test]
    fn missing_browser_is_explicit() {
        assert!(find_browser(Some("/nonexistent/buaa-browser"))
            .unwrap_err()
            .to_string()
            .contains("不可用"));
    }
    #[test]
    fn real_browser_waits_for_login_handler_before_clicking() {
        let Ok(executable) = find_browser(None) else {
            eprintln!("SKIP: no compatible browser installed");
            return;
        };
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let server = std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                if let Ok((mut stream, _)) = listener.accept() {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .unwrap();
                    let mut request = [0; 4096];
                    let count = stream.read(&mut request).unwrap_or(0);
                    let (content_type, body) = if request[..count]
                        .starts_with(b"GET /login-handler.js ")
                    {
                        // The actual portal renders its form before main.js binds #login-account.
                        std::thread::sleep(Duration::from_secs(2));
                        (
                            "application/javascript",
                            r#"setTimeout(() => { document.querySelector('#login-account').onclick = event => { window.fixtureSubmitted = event.isTrusted && document.querySelector('#username').value === 'fixture-user' && document.querySelector('#password').value === 'fixture-password'; if (window.fixtureSubmitted) document.body.innerHTML = '<button id="logout">注销</button>'; }; }, 250);"#,
                        )
                    } else {
                        (
                            "text/html; charset=utf-8",
                            include_str!("../../tests/fixtures/browser-loading.html"),
                        )
                    };
                    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nConnection: close\r\n\r\n{body}", body.len());
                } else {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        });
        let cancel =
            Cancellation::new(Arc::new(AtomicU64::new(0)), 0).with_timeout(Duration::from_secs(15));
        let mut browser = Browser::launch(&executable, None).unwrap();
        let profile = browser.profile.path().to_owned();
        let origin = format!("http://{address}");
        let mut cdp = browser.connect(&cancel, &origin).unwrap();
        let result = drive(
            &mut cdp,
            &cancel.with_timeout(Duration::from_secs(10)),
            &format!("{origin}/"),
            "fixture-user",
            "fixture-password",
            &|_| {},
        );
        let submitted = cdp.evaluate("window.fixtureSubmitted");
        let page = cdp.evaluate(include_str!("page.js"));
        drop(cdp);
        drop(browser);
        stopped.store(true, Ordering::SeqCst);
        server.join().unwrap();
        assert!(!profile.exists());
        assert_eq!(
            submitted.unwrap(),
            true,
            "login handler was not invoked: result={result:?}, page={page:?}"
        );
        result.unwrap();
    }
    #[test]
    fn real_browser_reaches_local_proxy_with_destination_dns_blocked() {
        let Ok(executable) = find_browser(None) else {
            eprintln!("SKIP: no compatible browser installed");
            return;
        };
        // Reject CONNECT locally: reaching this server must not require gateway access.
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if let Ok((mut stream, _)) = listener.accept() {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .unwrap();
                    stream
                        .set_write_timeout(Some(Duration::from_secs(1)))
                        .unwrap();
                    let mut header = Vec::new();
                    while header.len() < 8192 && !header.ends_with(b"\r\n\r\n") {
                        let mut byte = [0];
                        match stream.read(&mut byte) {
                            Ok(1) => header.push(byte[0]),
                            _ => break,
                        }
                    }
                    let _ = stream.write_all(
                        b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                    if header.starts_with(b"CONNECT gw.buaa.edu.cn:443 HTTP/1.1\r\n") {
                        return true;
                    }
                } else {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            false
        });
        let generation = Arc::new(AtomicU64::new(0));
        let cancel = Cancellation::new(generation, 0).with_timeout(Duration::from_secs(15));
        let mut browser = Browser::launch(&executable, Some(address)).unwrap();
        let profile = browser.profile.path().to_owned();
        let mut cdp = browser.connect(&cancel, "https://gw.buaa.edu.cn").unwrap();
        let navigation = cdp.call("Page.navigate", json!({"url":"https://gw.buaa.edu.cn/"}));
        let reached_proxy = server.join().unwrap();
        drop(cdp);
        drop(browser);
        assert!(!profile.exists());
        assert!(reached_proxy, "browser could not reach its loopback proxy");
        assert!(navigation.unwrap().get("errorText").is_some());
    }
    #[test]
    fn real_browser_clicks_logout_and_login_and_is_cleaned_up() {
        let Ok(executable) = find_browser(None) else {
            eprintln!("SKIP: no compatible browser installed");
            return;
        };
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let server = std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                if let Ok((mut stream, _)) = listener.accept() {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .unwrap();
                    let mut request = [0; 4096];
                    let _ = stream.read(&mut request);
                    let body = include_str!("../../tests/fixtures/browser.html");
                    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n{body}", body.len());
                } else {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        });
        let generation = Arc::new(AtomicU64::new(0));
        let cancel =
            Cancellation::new(generation.clone(), 0).with_timeout(Duration::from_secs(120));
        let mut browser = Browser::launch(&executable, None).unwrap();
        let profile = browser.profile.path().to_owned();
        let origin = format!("http://{address}");
        let mut cdp = browser.connect(&cancel, &origin).unwrap();
        let result = drive(
            &mut cdp,
            &cancel,
            &format!("{origin}/"),
            "fixture-user",
            "fixture-password",
            &|_| {},
        );
        let outcome = cdp.evaluate("window.fixtureResult");
        let scenario = |name: &str, limit: Duration| {
            let mut browser = Browser::launch(&executable, None).unwrap();
            let profile = browser.profile.path().to_owned();
            let mut cdp = browser.connect(&cancel, &origin).unwrap();
            let messages = RefCell::new(Vec::new());
            let result = drive(
                &mut cdp,
                &cancel.with_timeout(limit),
                &format!("{origin}/{name}#{name}"),
                "fixture-user",
                "fixture-password",
                &|message| messages.borrow_mut().push(message.to_owned()),
            );
            let page = cdp.evaluate(include_str!("page.js"));
            let outcome = cdp.evaluate("window.fixtureResult");
            drop(cdp);
            drop(browser);
            assert!(!profile.exists());
            (result, page, outcome, messages.into_inner())
        };
        let (direct_login, _, direct_outcome, direct_messages) =
            scenario("login", Duration::from_secs(10));
        let (mixed, _, mixed_outcome, mixed_messages) = scenario("mixed", Duration::from_secs(10));
        let (logout_stuck, _, stuck_outcome, stuck_messages) =
            scenario("logout-stuck", Duration::from_secs(3));
        let (logout_rejected, _, logout_error_outcome, _) =
            scenario("logout-rejected", Duration::from_secs(10));
        let (delayed, delayed_page, _, _) = scenario("delayed", Duration::from_secs(15));
        let (rejected, _, rejected_outcome, _) = scenario("rejected", Duration::from_secs(10));
        let (silent, _, _, _) = scenario("silent", Duration::from_secs(7));
        cdp.evaluate("(() => { document.body.innerHTML = '<input id=\"username\"><input id=\"password\" type=\"password\"><button id=\"login\">登录</button><button id=\"login\">登录</button>'; return true; })()").unwrap();
        assert_eq!(
            cdp.evaluate(include_str!("page.js")).unwrap()["state"],
            "ambiguous"
        );
        cdp.evaluate("(() => { document.body.innerHTML = '<input name=\"captcha\" placeholder=\"验证码\">'; return true; })()").unwrap();
        assert_eq!(
            cdp.evaluate(include_str!("page.js")).unwrap()["state"],
            "needs_user"
        );
        generation.fetch_add(1, Ordering::SeqCst);
        assert!(cdp.evaluate("true").is_err());
        drop(cdp);
        drop(browser);
        stopped.store(true, Ordering::SeqCst);
        server.join().unwrap();
        assert!(!profile.exists());
        result.unwrap();
        let outcome = outcome.unwrap();
        assert_eq!(outcome["logout"], true);
        assert_eq!(outcome["login"], true);
        assert_eq!(outcome["trusted"], true);
        assert_eq!(outcome["earlyInput"], false);
        assert_eq!(
            outcome["order"],
            json!(["logout", "confirm", "logged_out", "login", "online"])
        );
        direct_login.unwrap();
        let direct_outcome = direct_outcome.unwrap();
        assert_eq!(direct_outcome["order"], json!(["login", "online"]));
        assert_eq!(direct_outcome["logout"], false);
        assert!(direct_messages
            .iter()
            .any(|message| message.contains("检测到登录页面")));
        mixed.unwrap();
        let mixed_outcome = mixed_outcome.unwrap();
        assert_eq!(mixed_outcome["order"], outcome["order"]);
        assert_eq!(mixed_outcome["earlyInput"], false);
        assert_eq!(mixed_outcome["logoutClicks"], 1);
        let completed = mixed_messages
            .iter()
            .position(|message| message.contains("注销完成"))
            .unwrap();
        let submitted = mixed_messages
            .iter()
            .position(|message| message.contains("已提交网页登录"))
            .unwrap();
        assert!(completed < submitted);
        assert!(logout_stuck.is_err());
        let stuck_outcome = stuck_outcome.unwrap();
        assert_eq!(stuck_outcome["order"], json!(["logout", "confirm"]));
        assert_eq!(stuck_outcome["earlyInput"], false);
        assert_eq!(stuck_outcome["logoutClicks"], 1);
        assert!(stuck_messages
            .iter()
            .all(|message| !message.contains("已提交网页登录")));
        assert!(logout_rejected
            .unwrap_err()
            .to_string()
            .contains("认证页面返回错误"));
        let logout_error_outcome = logout_error_outcome.unwrap();
        assert_eq!(logout_error_outcome["order"], json!(["logout", "confirm"]));
        assert_ne!(logout_error_outcome["errorDismissed"], true);
        delayed.unwrap();
        assert_eq!(delayed_page.unwrap()["state"], "online");
        let rejection = rejected.unwrap_err();
        assert_eq!(rejection.kind, crate::transport::AuthErrorKind::NeedsUser);
        assert!(rejection.to_string().contains("E2531"));
        assert_ne!(rejected_outcome.unwrap()["errorDismissed"], true);
        assert!(
            silent.is_err(),
            "submitting a login form is not authentication success"
        );
    }
}
