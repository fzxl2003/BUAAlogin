//! External reachability is independent of the portal's accounting record.
use crate::{cancellation::Cancellation, interfaces::Interface, transport};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Authentication {
    #[default]
    Unknown,
    Offline,
    Online,
}
impl Authentication {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "未知",
            Self::Offline => "未认证",
            Self::Online => "已认证",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Connectivity {
    #[default]
    Unknown,
    Available,
    Unavailable,
}
impl Connectivity {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "未知",
            Self::Available => "可用",
            Self::Unavailable => "不可用",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeOutcome {
    Available,
    Http(u32),
    Dns,
    Tls,
    Timeout,
    Transport,
    Cancelled,
}
#[derive(Clone, Debug, Serialize)]
pub struct ProbeResult {
    pub endpoint: &'static str,
    pub outcome: ProbeOutcome,
}
#[derive(Clone, Debug, Serialize)]
pub struct ProbeReport {
    pub connectivity: Connectivity,
    pub results: Vec<ProbeResult>,
}
impl ProbeReport {
    pub fn from_results(results: Vec<ProbeResult>) -> Self {
        let connectivity = if results.iter().any(|r| r.outcome == ProbeOutcome::Available) {
            Connectivity::Available
        } else if results.iter().any(|r| r.outcome == ProbeOutcome::Cancelled) {
            Connectivity::Unknown
        } else {
            Connectivity::Unavailable
        };
        Self {
            connectivity,
            results,
        }
    }
    pub fn available(&self) -> bool {
        self.connectivity == Connectivity::Available
    }
    pub fn summary(&self) -> String {
        self.results
            .iter()
            .map(|r| format!("{}={:?}", r.endpoint, r.outcome))
            .collect::<Vec<_>>()
            .join("，")
    }
}
pub(crate) fn probe(interface: &Interface, cancel: &Cancellation) -> ProbeReport {
    // Separate easy handles: portal cookies, pinned resolution and GET settings
    // cannot leak into probes. Both bounded requests share one cancellation epoch.
    std::thread::scope(|scope| {
        let handles: Vec<_> = [
            ("百度", "https://www.baidu.com/"),
            ("腾讯", "https://www.qq.com/"),
        ]
        .into_iter()
        .map(|(endpoint, url)| {
            scope.spawn(move || ProbeResult {
                endpoint,
                outcome: transport::probe_head(interface, cancel, url),
            })
        })
        .collect();
        ProbeReport::from_results(
            handles
                .into_iter()
                .map(|h| {
                    h.join().unwrap_or(ProbeResult {
                        endpoint: "检测",
                        outcome: ProbeOutcome::Transport,
                    })
                })
                .collect(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn report(outcomes: &[ProbeOutcome]) -> ProbeReport {
        ProbeReport::from_results(
            outcomes
                .iter()
                .map(|o| ProbeResult {
                    endpoint: "fixture",
                    outcome: o.clone(),
                })
                .collect(),
        )
    }
    #[test]
    fn single_endpoint_failure_and_captive_redirect() {
        assert!(report(&[ProbeOutcome::Dns, ProbeOutcome::Available]).available());
        assert!(!report(&[ProbeOutcome::Http(302), ProbeOutcome::Tls]).available());
        assert_eq!(
            report(&[ProbeOutcome::Cancelled]).connectivity,
            Connectivity::Unknown
        );
    }
}
