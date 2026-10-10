use crate::{interfaces::Interface, protocol, Config};
use curl::easy::{Easy2, Handler, List, WriteError};
use regex::Regex;
use serde_json::Value;
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use url::Url;

const BASE: &str = "https://gw.buaa.edu.cn/";
struct Transfer {
    bytes: Vec<u8>,
    index: u32,
    generation: Arc<AtomicU64>,
    active: u64,
}
impl Handler for Transfer {
    fn write(&mut self, chunk: &[u8]) -> Result<usize, WriteError> {
        if self.bytes.len() + chunk.len() > 1_048_576 {
            return Ok(0);
        }
        self.bytes.extend_from_slice(chunk);
        Ok(chunk.len())
    }
    fn progress(&mut self, _: f64, _: f64, _: f64, _: f64) -> bool {
        self.generation.load(Ordering::SeqCst) == self.active
    }
    fn open_socket(
        &mut self,
        family: i32,
        socktype: i32,
        protocol: i32,
    ) -> Option<curl_sys::curl_socket_t> {
        let socket =
            socket2::Socket::new(family.into(), socktype.into(), Some(protocol.into())).ok()?;
        // Source address binding alone can follow the wrong default route.
        // Pin the outbound interface as well, and fail closed if unsupported.
        #[cfg(unix)]
        {
            use std::os::fd::{AsRawFd, IntoRawFd};
            #[cfg(target_os = "macos")]
            let (option, value) = (libc::IP_BOUND_IF, self.index);
            #[cfg(target_os = "linux")]
            let (option, value) = (libc::IP_UNICAST_IF, self.index.to_be());
            if unsafe {
                libc::setsockopt(
                    socket.as_raw_fd(),
                    libc::IPPROTO_IP,
                    option,
                    &value as *const _ as *const _,
                    std::mem::size_of_val(&value) as _,
                )
            } != 0
            {
                return None;
            }
            Some(socket.into_raw_fd())
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::{AsRawSocket, IntoRawSocket};
            use windows_sys::Win32::Networking::WinSock::{setsockopt, IPPROTO_IP, IP_UNICAST_IF};
            let value = self.index.to_be();
            if unsafe {
                setsockopt(
                    socket.as_raw_socket() as _,
                    IPPROTO_IP,
                    IP_UNICAST_IF,
                    &value as *const _ as _,
                    4,
                )
            } != 0
            {
                return None;
            }
            Some(socket.into_raw_socket() as _)
        }
    }
}

pub trait Portal {
    fn login(&mut self, username: &str, password: &str) -> Result<bool, String>;
    fn inspect(&mut self) -> Result<(), String>;
    fn online(&mut self) -> Result<bool, String>;
    fn campus_ip(&self) -> Option<&str>;
}
pub struct Session {
    easy: Easy2<Transfer>,
    ip: Option<String>,
    acid: Option<String>,
    generation: Arc<AtomicU64>,
    active_generation: u64,
    #[cfg(test)]
    script: Option<std::collections::VecDeque<(String, Option<Url>)>>,
    #[cfg(test)]
    requests: Vec<Url>,
}
impl Session {
    pub fn new(
        interface: Interface,
        config: &Config,
        generation: Arc<AtomicU64>,
    ) -> Result<Self, String> {
        let mut easy = Easy2::new(Transfer {
            bytes: Vec::new(),
            index: interface.index,
            generation: generation.clone(),
            active: generation.load(Ordering::SeqCst),
        });
        let setup = (|| -> Result<(), curl::Error> {
            easy.proxy("")?;
            easy.noproxy("*")?;
            easy.ip_resolve(curl::easy::IpResolve::V4)?;
            easy.interface(&interface.address.to_string())?;
            easy.connect_timeout(Duration::from_secs(3))?;
            easy.timeout(Duration::from_secs(15))?;
            easy.ssl_verify_peer(true)?;
            easy.ssl_verify_host(true)?;
            easy.follow_location(false)?;
            easy.cookie_file("")?;
            easy.progress(true)?;
            easy.tcp_keepalive(false)?;
            easy.signal(false)?;
            let mut hosts = List::new();
            hosts.append(&format!("gw.buaa.edu.cn:443:{}", config.gateway_ip))?;
            easy.resolve(hosts)?;
            Ok(())
        })();
        setup.map_err(|_| "无法配置校园网请求")?;
        Ok(Self {
            easy,
            ip: None,
            acid: None,
            active_generation: generation.load(Ordering::SeqCst),
            generation,
            #[cfg(test)]
            script: None,
            #[cfg(test)]
            requests: vec![],
        })
    }
    fn request(&mut self, url: &Url) -> Result<(String, Option<Url>), String> {
        validate_url(url)?;
        if self.generation.load(Ordering::SeqCst) != self.active_generation {
            return Err("已取消".into());
        }
        #[cfg(test)]
        {
            self.requests.push(url.clone());
            if let Some(script) = self.script.as_mut() {
                return script.pop_front().ok_or_else(|| "测试响应耗尽".into());
            }
        }
        self.easy.url(url.as_str()).map_err(|_| "请求地址错误")?;
        self.easy.get_mut().bytes.clear();
        self.easy.get_mut().active = self.active_generation;
        self.easy
            .perform()
            .map_err(|_| "校园网请求失败；检查接口、网关和证书")?;
        let bytes = std::mem::take(&mut self.easy.get_mut().bytes);
        let status = self.easy.response_code().map_err(|_| "HTTP 状态读取失败")?;
        let redirect = if [301, 302, 303, 307, 308].contains(&status) {
            let target = self
                .easy
                .redirect_url()
                .map_err(|_| "重定向读取失败")?
                .ok_or("重定向地址缺失")?;
            let target = url.join(target).map_err(|_| "重定向地址错误")?;
            validate_url(&target)?;
            Some(target)
        } else if status == 200 {
            None
        } else {
            return Err(format!("网关 HTTP 错误 {status}"));
        };
        Ok((
            String::from_utf8(bytes).map_err(|_| "网关编码错误")?,
            redirect,
        ))
    }
    fn api(&mut self, path: &str, params: &[(&str, &str)]) -> Result<Value, String> {
        let mut url = Url::parse(BASE).unwrap().join(path).unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        url.query_pairs_mut()
            .extend_pairs(params.iter().copied())
            .append_pair("callback", &format!("buaa{}", now.as_nanos()))
            .append_pair("_", &now.as_millis().to_string());
        let (body, redirect) = self.request(&url)?;
        if redirect.is_some() {
            return Err("认证接口发生重定向".into());
        }
        protocol::parse_jsonp(&body)
    }
    pub fn activate(&mut self) {
        self.active_generation = self.generation.load(Ordering::SeqCst);
    }
}
pub fn validate_url(url: &Url) -> Result<(), String> {
    if url.scheme() != "https"
        || url.host_str() != Some("gw.buaa.edu.cn")
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("认证请求和重定向必须使用同一网关的 HTTPS".into());
    }
    Ok(())
}
impl Portal for Session {
    fn inspect(&mut self) -> Result<(), String> {
        let mut url = Url::parse(BASE).unwrap();
        let refresh = Regex::new(r#"(?i)<meta\b[^>]*content=['"][^'"]*url=([^'"]+)"#).unwrap();
        let ip_re = Regex::new(r#"\bip\s*:\s*['"]([^'"]+)"#).unwrap();
        let input_re = Regex::new(r#"id=['"]user_ip['"]\s+value=['"]([^'"]+)"#).unwrap();
        let acid_re = Regex::new(r#"\bacid\s*:\s*['"]([^'"]+)"#).unwrap();
        for _ in 0..6 {
            let (body, redirect) = self.request(&url)?;
            if let Some(next) = redirect {
                url = next;
                continue;
            }
            if let Some(c) = refresh.captures(&body) {
                url = url
                    .join(&c[1].replace("&amp;", "&"))
                    .map_err(|_| "页面跳转地址错误")?;
                continue;
            }
            let ip = ip_re
                .captures(&body)
                .or_else(|| input_re.captures(&body))
                .ok_or("页面没有校园网 IP")?[1]
                .to_string();
            let ip = ip
                .parse::<std::net::Ipv4Addr>()
                .map_err(|_| "网关没有返回有效的校园网 IPv4")?;
            if ip.is_unspecified() || ip.is_loopback() || ip.is_multicast() || ip.is_broadcast() {
                return Err("网关没有返回有效的校园网 IPv4".into());
            }
            let ip = ip.to_string();
            let acid = acid_re
                .captures(&body)
                .map(|c| c[1].to_string())
                .or_else(|| {
                    url.query_pairs()
                        .find(|(k, _)| k == "ac_id")
                        .map(|(_, v)| v.into_owned())
                })
                .ok_or("页面没有有效 ac_id")?;
            if acid.is_empty() || !acid.bytes().all(|c| c.is_ascii_digit()) {
                return Err("页面没有有效 ac_id".into());
            }
            self.ip = Some(ip);
            self.acid = Some(acid);
            return Ok(());
        }
        Err("网关重定向次数过多".into())
    }
    fn online(&mut self) -> Result<bool, String> {
        let data = self.api("/cgi-bin/rad_user_info", &[])?;
        Ok(data["error"] == "ok"
            && data["online_ip"]
                .as_str()
                .is_some_and(|ip| self.ip.as_deref() == Some(ip)))
    }
    fn campus_ip(&self) -> Option<&str> {
        self.ip.as_deref()
    }
    fn login(&mut self, username: &str, password: &str) -> Result<bool, String> {
        if self.ip.is_some() && self.online()? {
            return Ok(false);
        }
        self.inspect()?;
        if self.online()? {
            return Ok(false);
        }
        let ip = self.ip.clone().unwrap();
        let acid = self.acid.clone().unwrap();
        let challenge = self.api(
            "/cgi-bin/get_challenge",
            &[("username", username), ("ip", &ip)],
        )?;
        let token = challenge["challenge"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or("获取 challenge 失败")?;
        if challenge["client_ip"].as_str().is_some_and(|v| v != ip) {
            return Err("challenge IP 与网关报告的校园网 IP 不一致".into());
        }
        let fields = protocol::login_fields(username, password, &ip, &acid, token);
        let reply = self.api(
            "/cgi-bin/srun_portal",
            &[
                ("action", "login"),
                ("username", username),
                ("password", &fields.password),
                ("ac_id", &acid),
                ("ip", &ip),
                ("info", &fields.info),
                ("chksum", &fields.checksum),
                ("n", "200"),
                ("type", "1"),
                ("double_stack", "0"),
                ("os", std::env::consts::OS),
                ("name", "BUAALogin"),
            ],
        )?;
        if reply["error"] != "ok" {
            // Gateway error messages can echo secrets; emit only a bounded error code.
            let code = reply["error"].as_str().unwrap_or("unknown");
            let safe = if code.len() <= 40
                && code.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
            {
                code
            } else {
                "unknown"
            };
            return Err(format!("登录失败：{safe}"));
        }
        if !self.online()? {
            return Err("网关接受认证，但未确认校园网 IP 在线".into());
        }
        Ok(true)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redirects_are_pinned() {
        assert!(validate_url(&Url::parse("https://gw.buaa.edu.cn/x").unwrap()).is_ok());
        for url in [
            "http://gw.buaa.edu.cn/",
            "https://other.example/",
            "https://gw.buaa.edu.cn:444/",
            "https://user@gw.buaa.edu.cn/",
        ] {
            assert!(validate_url(&Url::parse(url).unwrap()).is_err());
        }
    }
    fn fixture(bodies: &[&str]) -> Session {
        let i = Interface {
            name: "fixture".into(),
            address: "10.1.2.3".parse().unwrap(),
            index: 2,
        };
        let mut session = Session::new(i, &Config::default(), Arc::new(AtomicU64::new(0))).unwrap();
        session.script = Some(bodies.iter().map(|b| (b.to_string(), None)).collect());
        session
    }
    #[test]
    fn complete_login_and_online_fast_path() {
        let mut s = fixture(&[
            r#"var CONFIG = {acid: "78", ip: "10.1.2.3"};"#,
            r#"{"error":"not_online_error"}"#,
            r#"{"challenge":"abc123","client_ip":"10.1.2.3"}"#,
            r#"{"error":"ok"}"#,
            r#"{"error":"ok","online_ip":"10.1.2.3"}"#,
            r#"{"error":"ok","online_ip":"10.1.2.3"}"#,
        ]);
        assert!(s.login("test", "password").unwrap());
        assert_eq!(s.requests.len(), 5);
        let query = s.requests[3]
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(query.get("ac_id").unwrap(), "78");
        assert!(query.get("password").unwrap().starts_with("{MD5}"));
        assert!(!s.login("test", "password").unwrap());
        assert_eq!(s.requests.len(), 6);
        assert_eq!(s.requests[5].path(), "/cgi-bin/rad_user_info");
    }
    #[test]
    fn modern_meta_redirect_and_router_nat() {
        let mut s = fixture(&[
            r#"<meta content="0;url=/srun_portal_pc?ac_id=78&amp;theme=buaa">"#,
            r#"var CONFIG = {ip: "10.1.2.3"};"#,
        ]);
        s.inspect().unwrap();
        assert_eq!(s.acid.as_deref(), Some("78"));
        assert!(s.requests[1].as_str().contains("ac_id=78&theme=buaa"));
        let mut s = fixture(&[r#"var CONFIG = {acid: "78", ip: "172.20.10.2"};"#]);
        s.inspect().unwrap();
        assert_eq!(s.campus_ip(), Some("172.20.10.2"));
    }
    #[test]
    fn login_requires_online_confirmation() {
        let mut s = fixture(&[
            r#"var CONFIG = {acid: "78", ip: "10.1.2.3"};"#,
            r#"{"error":"not_online_error"}"#,
            r#"{"challenge":"abc123"}"#,
            r#"{"error":"ok"}"#,
            r#"{"error":"ok","online_ip":"172.20.10.2"}"#,
        ]);
        assert!(s.login("test", "password").unwrap_err().contains("未确认"));
    }
    #[test]
    fn cancelled_session_makes_no_followup_requests() {
        let mut s = fixture(&[]);
        s.generation.fetch_add(1, Ordering::SeqCst);
        assert!(s.inspect().is_err());
        assert!(s.requests.is_empty());
        assert!(!s.easy.get_mut().progress(0.0, 0.0, 0.0, 0.0));
    }
    #[test]
    fn challenge_must_match_interface() {
        let mut s = fixture(&[
            r#"var CONFIG = {acid: "78", ip: "10.1.2.3"};"#,
            r#"{"error":"not_online_error"}"#,
            r#"{"challenge":"abc123","client_ip":"172.20.10.2"}"#,
        ]);
        assert!(s.login("test", "password").unwrap_err().contains("不一致"));
        assert_eq!(s.requests.len(), 3);
    }
}
