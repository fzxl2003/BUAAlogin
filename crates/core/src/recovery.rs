//! Policy is separate from network I/O so time, escalation and shared exits can
//! be tested without logging in or waiting on the campus gateway.
use crate::transport::{AuthError, AuthErrorKind};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryMode {
    Protocol,
    Browser,
}
#[derive(Default)]
pub(crate) struct ExitState {
    failures: u32,
    last_round: Option<Instant>,
    last_recovery: Option<Instant>,
    pub blocked: Option<String>,
}
impl ExitState {
    pub fn failed(&mut self, now: Instant, interval: u64) -> bool {
        if self
            .last_round
            .is_none_or(|last| now.saturating_duration_since(last) >= Duration::from_secs(interval))
        {
            self.failures = self.failures.saturating_add(1);
            self.last_round = Some(now);
        }
        self.failures >= 3
    }
    pub fn healthy(&mut self) {
        self.failures = 0;
        self.last_round = None;
    }
    pub fn cooling(&self, now: Instant, interval: u64) -> bool {
        self.last_recovery
            .is_some_and(|last| now.saturating_duration_since(last) < Duration::from_secs(interval))
    }
    pub fn begin(&mut self, now: Instant, interval: u64) -> bool {
        if self.blocked.is_some() || self.cooling(now, interval) {
            return false;
        }
        // Reserve before any destructive operation; cancellation cannot erase it.
        self.last_recovery = Some(now);
        true
    }
    pub fn finish(&mut self, now: Instant) {
        self.last_recovery = Some(now);
    }
}
#[derive(Default)]
pub(crate) struct RecoveryBook {
    exits: HashMap<String, ExitState>,
}
impl RecoveryBook {
    pub fn exit(&mut self, gateway: std::net::Ipv4Addr, campus_ip: &str) -> &mut ExitState {
        self.exits
            .entry(format!("{gateway}/{campus_ip}"))
            .or_default()
    }
}

pub(crate) trait RecoveryOps {
    fn reset(&mut self) -> Result<(), AuthError>;
    fn probe(&mut self) -> Result<bool, AuthError>;
    fn browser(&mut self) -> Result<(), AuthError>;
    fn verification_detail(&self) -> Option<String> {
        None
    }
}
fn verification_error(ops: &impl RecoveryOps, message: String) -> AuthError {
    match ops.verification_detail() {
        Some(detail) => format!("{message}；{detail}").into(),
        None => message.into(),
    }
}
pub(crate) fn execute(
    ops: &mut impl RecoveryOps,
    mode: RecoveryMode,
    browser_enabled: bool,
    progress: &dyn Fn(&str, &str),
) -> Result<(), AuthError> {
    let mut protocol_error = None;
    if mode == RecoveryMode::Protocol {
        progress("protocol_recovery", "协议恢复中：注销并重新认证");
        if let Err(error) = ops.reset() {
            if matches!(
                error.kind,
                AuthErrorKind::NeedsUser | AuthErrorKind::Cancelled
            ) {
                return Err(error);
            }
            protocol_error = Some(error.to_string());
        }
        progress("verifying", "正在验证外网是否恢复");
        if ops.probe()? {
            return Ok(());
        }
    }
    if !browser_enabled {
        return Err(verification_error(
            ops,
            format!(
                "{}；外网仍不可用，网页兜底未启用",
                protocol_error.as_deref().unwrap_or("协议恢复已完成")
            ),
        ));
    }
    progress("browser_recovery", "网页兜底中：后台打开认证页面并模拟登录");
    ops.browser()?;
    progress("verifying", "网页步骤已执行，正在验证外网");
    if ops.probe()? {
        Ok(())
    } else {
        Err(verification_error(ops, "网页操作后外网仍不可用".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    #[test]
    fn failures_and_recovery_share_the_configured_interval() {
        let mut book = RecoveryBook::default();
        let gateway = "10.200.21.4".parse().unwrap();
        let now = Instant::now();
        let interval = 45;
        assert!(!book.exit(gateway, "10.1.1.1").failed(now, interval));
        assert!(!book.exit(gateway, "10.1.1.1").failed(now, interval));
        assert!(!book
            .exit(gateway, "10.1.1.1")
            .failed(now + Duration::from_secs(interval), interval));
        let exit = book.exit(gateway, "10.1.1.1");
        assert!(exit.failed(now + Duration::from_secs(interval * 2), interval));
        assert!(exit.begin(now, interval));
        // Recreating an interface Session never recreates the book.
        assert!(!book
            .exit(gateway, "10.1.1.1")
            .begin(now + Duration::from_secs(interval - 1), interval));
        assert!(book.exit(gateway, "10.1.1.2").begin(now, interval));
        assert!(book
            .exit(gateway, "10.1.1.1")
            .begin(now + Duration::from_secs(interval), interval));
        let exit = book.exit(gateway, "10.1.1.1");
        exit.healthy();
        assert!(!exit.failed(now + Duration::from_secs(interval * 3), interval));
        // A live setting change also changes the shared exit's recovery limit.
        assert!(exit.begin(now + Duration::from_secs(interval + 10), 10));
        let completed = now + Duration::from_secs(120);
        exit.finish(completed);
        assert!(!exit.begin(completed + Duration::from_secs(9), 10));
        assert!(exit.begin(completed + Duration::from_secs(10), 10));
    }
    struct Fake {
        calls: Vec<&'static str>,
        probes: VecDeque<bool>,
        error: Option<AuthError>,
        browser_error: Option<AuthError>,
        detail: Option<String>,
    }
    impl RecoveryOps for Fake {
        fn reset(&mut self) -> Result<(), AuthError> {
            self.calls.push("reset");
            self.error.take().map_or(Ok(()), Err)
        }
        fn probe(&mut self) -> Result<bool, AuthError> {
            self.calls.push("probe");
            Ok(self.probes.pop_front().unwrap_or(false))
        }
        fn browser(&mut self) -> Result<(), AuthError> {
            self.calls.push("browser");
            self.browser_error.take().map_or(Ok(()), Err)
        }
        fn verification_detail(&self) -> Option<String> {
            self.detail.clone()
        }
    }
    fn fake(probes: &[bool]) -> Fake {
        Fake {
            calls: vec![],
            probes: probes.iter().copied().collect(),
            error: None,
            browser_error: None,
            detail: None,
        }
    }
    #[test]
    fn escalation_and_external_confirmation() {
        let mut ops = fake(&[false, true]);
        execute(&mut ops, RecoveryMode::Protocol, true, &|_, _| {}).unwrap();
        assert_eq!(ops.calls, ["reset", "probe", "browser", "probe"]);
        let mut ops = fake(&[true]);
        execute(&mut ops, RecoveryMode::Protocol, true, &|_, _| {}).unwrap();
        assert!(!ops.calls.contains(&"browser"));
        let mut ops = fake(&[]);
        assert!(execute(&mut ops, RecoveryMode::Protocol, true, &|_, _| {}).is_err());
        assert_eq!(ops.calls.iter().filter(|c| **c == "reset").count(), 1);
        assert_eq!(ops.calls.iter().filter(|c| **c == "browser").count(), 1);
    }
    #[test]
    fn restriction_and_cancellation_do_not_escalate() {
        for error in [AuthError::needs_user("密码错误"), AuthError::from("已取消")] {
            let mut ops = fake(&[]);
            ops.error = Some(error);
            assert!(execute(&mut ops, RecoveryMode::Protocol, true, &|_, _| {}).is_err());
            assert_eq!(ops.calls, ["reset"]);
        }
        let mut ops = fake(&[true]);
        execute(&mut ops, RecoveryMode::Browser, true, &|_, _| {}).unwrap();
        assert_eq!(ops.calls, ["browser", "probe"]);
    }
    #[test]
    fn browser_failure_preserves_authentication_and_probe_results() {
        let mut ops = fake(&[false]);
        ops.detail = Some("网关认证：未认证；百度=Timeout，腾讯=Timeout".into());
        let error = execute(&mut ops, RecoveryMode::Browser, true, &|_, _| {}).unwrap_err();
        assert!(error.to_string().contains("网关认证：未认证"));
        assert!(error.to_string().contains("百度=Timeout，腾讯=Timeout"));
        assert_eq!(ops.calls.iter().filter(|c| **c == "browser").count(), 1);
        assert_eq!(ops.calls.iter().filter(|c| **c == "probe").count(), 1);
    }
}
