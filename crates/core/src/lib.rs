#[cfg(feature = "browser-fallback")]
mod browser;
mod cancellation;
pub mod connectivity;
pub mod engine;
pub mod ffi;
pub mod interfaces;
pub mod protocol;
mod recovery;
mod scheduler;
pub mod transport;

use serde::{Deserialize, Serialize};
use std::{collections::HashMap, net::Ipv4Addr};
use zeroize::Zeroizing;

#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub username: String,
    pub password: Zeroizing<String>,
    pub interface: Option<String>,
    /// Empty means automatic selection. This coexists with `interface` for
    /// backwards compatibility with older settings and the CLI.
    pub interfaces: Vec<String>,
    pub gateway_ip: Ipv4Addr,
    /// Optional gateway overrides keyed by interface binding/name/address.
    pub interface_gateways: HashMap<String, Ipv4Addr>,
    pub interval: u64,
    pub try_all: bool,
    /// macOS supplies a single-shot GCD timer instead of a worker timeout.
    pub external_timer: bool,
    pub portal_auth_enabled: bool,
    pub connectivity_probe_enabled: bool,
    pub browser_fallback: bool,
    pub browser_path: Option<String>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            username: String::new(),
            password: Zeroizing::new(String::new()),
            interface: None,
            interfaces: Vec::new(),
            gateway_ip: Ipv4Addr::new(10, 200, 21, 4),
            interface_gateways: HashMap::new(),
            interval: 300,
            try_all: false,
            external_timer: false,
            portal_auth_enabled: true,
            connectivity_probe_enabled: true,
            browser_fallback: cfg!(feature = "browser-fallback"),
            browser_path: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskScheduleReason {
    Disabled,
    OnStart,
    Scheduled,
    WaitingTrigger,
    Cooldown,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskSnapshot {
    pub enabled: bool,
    pub phase: String,
    pub message: String,
    pub last_run_at: Option<u64>,
    pub next_run_at: Option<u64>,
    pub schedule_reason: TaskScheduleReason,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskSnapshots {
    pub portal_auth: TaskSnapshot,
    pub connectivity: TaskSnapshot,
    pub browser_fallback: TaskSnapshot,
}
impl Config {
    pub fn monitors_all_selected(&self) -> bool {
        self.try_all || self.interfaces.len() > 1
    }

    pub fn gateway_for(&self, interface: &interfaces::Interface) -> Ipv4Addr {
        self.interface_gateways
            .get(&interface.binding())
            .or_else(|| self.interface_gateways.get(&interface.name))
            .or_else(|| self.interface_gateways.get(&interface.address.to_string()))
            .copied()
            .unwrap_or(self.gateway_ip)
    }

    pub fn validate(&self, credentials: bool) -> Result<(), String> {
        if credentials
            && (self.portal_auth_enabled || self.browser_fallback)
            && (self.username.trim().is_empty() || self.password.is_empty())
        {
            return Err("账号和密码不能为空".into());
        }
        if !(10..=86400).contains(&self.interval) {
            return Err("检查间隔须在 10–86400 秒之间".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod config_tests {
    use super::*;

    #[test]
    fn old_config_defaults_to_both_scheduled_tasks_enabled() {
        let config: Config = serde_json::from_str("{}").unwrap();
        assert!(config.portal_auth_enabled);
        assert!(config.connectivity_probe_enabled);
    }

    #[test]
    fn external_only_monitor_does_not_require_credentials() {
        let config = Config {
            portal_auth_enabled: false,
            connectivity_probe_enabled: true,
            browser_fallback: false,
            ..Config::default()
        };
        assert!(config.validate(true).is_ok());
    }
}

/// Read-only UI state; timestamps are Unix seconds and addresses are never credentials.
#[cfg(target_os = "windows")]
#[derive(Debug, Clone, Copy, Serialize)]
pub struct TrafficDetails {
    pub used_bytes: u64,
    pub total_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DashboardSnapshot {
    pub phase: String,
    pub interface: Option<String>,
    pub local_ip: Option<Ipv4Addr>,
    pub campus_ip: Option<String>,
    pub traffic_used_bytes: Option<u64>,
    /// Windows display metadata from the existing status response, without extra requests.
    #[cfg(target_os = "windows")]
    pub traffic_details: Option<TrafficDetails>,
    pub last_check_at: Option<u64>,
    pub retry_at: Option<u64>,
    pub interval: u64,
    pub authentication: connectivity::Authentication,
    pub connectivity: connectivity::Connectivity,
    pub recovery_stage: Option<String>,
    pub tasks: TaskSnapshots,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authentication: Option<connectivity::Authentication>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connectivity: Option<connectivity::Connectivity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probes: Option<Vec<connectivity::ProbeResult>>,
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
            authentication: None,
            connectivity: None,
            probes: None,
        }
    }
}
