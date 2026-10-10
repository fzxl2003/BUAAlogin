use std::process::Command;
#[test]
fn credentials_required_before_network_even_for_status() {
    for args in [vec![], vec!["--status"], vec!["--once"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_buaalogin"))
            .args(args)
            .env_remove("USERNAME")
            .env_remove("PASSWORD")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr).contains("账号和密码不能为空"));
        assert!(output.stdout.is_empty());
    }
}
#[test]
fn invalid_interval_fails_without_exposing_credentials() {
    let output = Command::new(env!("CARGO_BIN_EXE_buaalogin"))
        .args(["--interval", "0"])
        .env("USERNAME", "secret-user")
        .env("PASSWORD", "secret-password")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("检查间隔"));
    assert!(!stderr.contains("secret"));
}
#[cfg(unix)]
#[test]
fn sigterm_interrupts_waiting_for_network() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_buaalogin"))
        .args(["--interface", "nonexistent-test-interface"])
        .env("USERNAME", "test")
        .env("PASSWORD", "test")
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::BufRead;
    let mut reader = std::io::BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(line.contains("waiting_network"));
    unsafe {
        libc::kill(child.id() as _, libc::SIGTERM);
    }
    assert!(child.wait().unwrap().success());
}
