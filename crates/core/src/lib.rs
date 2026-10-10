pub mod engine;
pub mod ffi;
pub mod interfaces;
pub mod protocol;
mod scheduler;
pub mod transport;

use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;
use zeroize::Zeroizing;

#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub username: String,
    pub password: Zeroizing<String>,
    pub interface: Option<String>,
    pub gateway_ip: Ipv4Addr,
    pub interval: u64,
    pub try_all: bool,
    /// macOS supplies a single-shot GCD timer instead of a worker timeout.
    pub external_timer: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            username: String::new(),
            password: Zeroizing::new(String::new()),
            interface: None,
            gateway_ip: Ipv4Addr::new(10, 200, 21, 4),
            interval: 300,
            try_all: false,
            external_timer: false,
        }
    }
}
impl Config {
    pub fn validate(&self, credentials: bool) -> Result<(), String> {
        if credentials && (self.username.trim().is_empty() || self.password.is_empty()) {
            return Err("账号和密码不能为空".into());
        }
        if !(10..=86400).contains(&self.interval) {
            return Err("检查间隔须在 10–86400 秒之间".into());
        }
        Ok(())
    }
}

/// Read-only UI state; timestamps are Unix seconds and addresses are never credentials.
#[derive(Debug, Clone, Serialize)]
pub struct DashboardSnapshot {
    pub phase: String,
    pub interface: Option<String>,
    pub local_ip: Option<Ipv4Addr>,
    pub campus_ip: Option<String>,
    pub last_check_at: Option<u64>,
    pub retry_at: Option<u64>,
    pub interval: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub state: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub records: Option<Vec<interfaces::InterfaceRecord>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delay: Option<u64>,
    /// Unix timestamp of the next automatic retry, for event-driven UI display.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<u64>,
    pub leeway: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dashboard: Option<DashboardSnapshot>,
}
impl Event {
    pub fn new(state: &str, message: impl Into<String>) -> Self {
        Self {
            state: state.into(),
            message: message.into(),
            records: None,
            delay: None,
            retry_at: None,
            leeway: 0,
            dashboard: None,
        }
    }
}
