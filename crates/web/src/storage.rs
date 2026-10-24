use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::Write,
    net::Ipv4Addr,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroizing;

use crate::credentials;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    pub gateway_ip: String,
    pub interfaces: Vec<String>,
    pub interface_gateways: HashMap<String, String>,
    pub interval: u64,
    pub portal_auth_enabled: bool,
    pub connectivity_probe_enabled: bool,
    pub traffic_total_gb: String,
    pub monitor_enabled: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            gateway_ip: "10.200.21.4".into(),
            interfaces: vec![],
            interface_gateways: HashMap::new(),
            interval: 300,
            portal_auth_enabled: true,
            connectivity_probe_enabled: true,
            traffic_total_gb: String::new(),
            monitor_enabled: false,
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        self.gateway_ip
            .parse::<Ipv4Addr>()
            .map_err(|_| "默认网关必须是 IPv4 地址")?;
        if !(10..=86400).contains(&self.interval) {
            return Err("检查周期须为 10–86400 秒".into());
        }
        for value in self.interface_gateways.values() {
            value
                .parse::<Ipv4Addr>()
                .map_err(|_| "接口网关必须是 IPv4 地址")?;
        }
        if !self.traffic_total_gb.trim().is_empty()
            && self
                .traffic_total_gb
                .trim()
                .parse::<f64>()
                .ok()
                .is_none_or(|value| !value.is_finite() || value <= 0.0)
        {
            return Err("总流量须为大于 0 的数字，或留空".into());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub struct AccountFile {
    pub username: String,
    pub password: String,
}

#[derive(Clone)]
pub struct Account {
    pub username: String,
    pub password: Zeroizing<String>,
    pub remembered: bool,
}

#[derive(Clone)]
pub struct Storage {
    root: PathBuf,
}

impl Storage {
    pub fn new(root: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&root).map_err(|_| "无法创建数据目录")?;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
            .map_err(|_| "无法设置数据目录权限")?;
        let logs = root.join("logs");
        fs::create_dir_all(&logs).map_err(|_| "无法创建日志目录")?;
        fs::set_permissions(&logs, fs::Permissions::from_mode(0o700))
            .map_err(|_| "无法设置日志目录权限")?;
        Ok(Self { root })
    }

    pub fn settings_path(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    pub fn settings_exist(&self) -> bool {
        self.settings_path().exists()
    }

    pub fn load_settings(&self) -> Result<Settings, String> {
        let path = self.settings_path();
        if !path.exists() {
            return Ok(Settings::default());
        }
        let bytes = fs::read(path).map_err(|_| "无法读取设置")?;
        let settings: Settings = serde_json::from_slice(&bytes).map_err(|_| "设置文件格式错误")?;
        settings.validate()?;
        Ok(settings)
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<(), String> {
        settings.validate()?;
        self.atomic_write(
            &self.settings_path(),
            &serde_json::to_vec_pretty(settings).map_err(|_| "无法编码设置")?,
        )
    }

    pub fn load_account(&self) -> Result<Option<Account>, String> {
        let path = self.root.join("credentials.dat");
        if !path.exists() {
            return Ok(None);
        }
        let encrypted = fs::read(path).map_err(|_| "无法读取凭据")?;
        let mut plain = credentials::decrypt(&encrypted)?;
        let parsed = serde_json::from_slice::<AccountFile>(&plain).map_err(|_| "凭据文件格式错误");
        plain.fill(0);
        let parsed = parsed?;
        Ok(Some(Account {
            username: parsed.username,
            password: Zeroizing::new(parsed.password),
            remembered: true,
        }))
    }

    pub fn save_account(&self, account: &Account) -> Result<(), String> {
        if !account.remembered {
            return self.delete_account();
        }
        let mut plain = serde_json::to_vec(&AccountFile {
            username: account.username.clone(),
            password: account.password.to_string(),
        })
        .map_err(|_| "无法编码凭据")?;
        let encrypted = credentials::encrypt(&plain);
        plain.fill(0);
        self.atomic_write(&self.root.join("credentials.dat"), &encrypted)
    }

    pub fn delete_account(&self) -> Result<(), String> {
        let path = self.root.join("credentials.dat");
        if path.exists() {
            fs::remove_file(path).map_err(|_| "无法删除凭据")?;
        }
        Ok(())
    }

    pub fn auth_path(&self) -> PathBuf {
        self.root.join("web-auth.json")
    }

    pub fn read_auth_hash(&self) -> Result<Option<String>, String> {
        let path = self.auth_path();
        if !path.exists() {
            return Ok(None);
        }
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(path).map_err(|_| "无法读取网页认证设置")?)
                .map_err(|_| "网页认证设置格式错误")?;
        Ok(value["password_hash"].as_str().map(str::to_owned))
    }

    pub fn write_auth_hash(&self, hash: &str) -> Result<(), String> {
        self.atomic_write(
            &self.auth_path(),
            &serde_json::to_vec_pretty(&serde_json::json!({"password_hash": hash}))
                .map_err(|_| "无法编码网页认证设置")?,
        )
    }

    pub fn append_log(&self, line: &str) -> Result<(), String> {
        let path = self.root.join("logs").join(format!("{}.log", day_string()));
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)
            .map_err(|_| "无法写入日志")?;
        writeln!(file, "{line}").map_err(|_| "无法写入日志".into())
    }

    pub fn current_log(&self) -> Result<Vec<u8>, String> {
        let path = self.root.join("logs").join(format!("{}.log", day_string()));
        if path.exists() {
            fs::read(path).map_err(|_| "无法读取日志".into())
        } else {
            Ok(Vec::new())
        }
    }

    pub fn cleanup_logs(&self) {
        let cutoff = SystemTime::now().checked_sub(Duration::from_secs(30 * 86400));
        let Ok(entries) = fs::read_dir(self.root.join("logs")) else {
            return;
        };
        for entry in entries.flatten() {
            if cutoff.is_some_and(|cutoff| {
                entry
                    .metadata()
                    .and_then(|metadata| metadata.modified())
                    .is_ok_and(|modified| modified < cutoff)
            }) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    fn atomic_write(&self, path: &Path, bytes: &[u8]) -> Result<(), String> {
        let tmp = path.with_extension("tmp");
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|_| "无法保存数据")?;
        file.write_all(bytes).map_err(|_| "无法保存数据")?;
        file.sync_all().map_err(|_| "无法同步数据")?;
        fs::rename(&tmp, path).map_err(|_| "无法替换数据文件")?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|_| "无法设置数据文件权限".to_string())
    }
}

pub fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn day_string() -> String {
    unsafe {
        let now = timestamp() as libc::time_t;
        let mut output: libc::tm = std::mem::zeroed();
        if !libc::localtime_r(&now, &mut output).is_null() {
            return format!(
                "{:04}-{:02}-{:02}",
                output.tm_year + 1900,
                output.tm_mon + 1,
                output.tm_mday
            );
        }
    }
    (timestamp() / 86400).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST: AtomicU64 = AtomicU64::new(1);

    fn temporary_storage() -> Storage {
        let unique = NEXT_TEST.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "buaa-web-test-{}-{}-{unique}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&path);
        Storage::new(path).unwrap()
    }

    #[test]
    fn settings_and_credentials_round_trip() {
        let storage = temporary_storage();
        let mut settings = Settings::default();
        settings.interfaces = vec!["eth0".into(), "wlan0".into()];
        storage.save_settings(&settings).unwrap();
        assert_eq!(storage.load_settings().unwrap().interfaces.len(), 2);
        let account = Account {
            username: "test".into(),
            password: Zeroizing::new("secret".into()),
            remembered: true,
        };
        storage.save_account(&account).unwrap();
        let loaded = storage.load_account().unwrap().unwrap();
        assert_eq!(loaded.username, "test");
        assert_eq!(loaded.password.as_str(), "secret");
        let _ = fs::remove_dir_all(storage.root);
    }

    #[test]
    fn rejects_invalid_settings() {
        let mut settings = Settings::default();
        settings.interval = 1;
        assert!(settings.validate().is_err());
        settings.interval = 300;
        settings.traffic_total_gb = "nope".into();
        assert!(settings.validate().is_err());
    }

    #[test]
    fn rejects_corrupt_persisted_files() {
        let storage = temporary_storage();
        storage
            .atomic_write(&storage.settings_path(), b"not-json")
            .unwrap();
        assert!(storage.load_settings().is_err());
        storage
            .atomic_write(&storage.root.join("credentials.dat"), b"not-credentials")
            .unwrap();
        assert!(storage.load_account().is_err());
        let _ = fs::remove_dir_all(storage.root);
    }
}
