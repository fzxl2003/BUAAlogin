//! Native WLAN API, event-driven discovery and association; no polling timer.
use std::{
    collections::{HashMap, HashSet},
    ptr::{null, null_mut},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use windows_sys::{
    core::GUID,
    Win32::{
        Foundation::{HANDLE, HWND},
        NetworkManagement::WiFi::*,
        UI::WindowsAndMessaging::PostMessageW,
    },
};
const SSID: &[u8] = b"BUAA-WiFi";
#[derive(Clone)]
pub struct Adapter {
    pub id: String,
    pub name: String,
    guid: GUID,
}
fn id(g: &GUID) -> String {
    format!(
        "{:08x}-{:04x}-{:04x}-{}",
        g.data1,
        g.data2,
        g.data3,
        g.data4
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
}
fn string(chars: &[u16]) -> String {
    String::from_utf16_lossy(&chars[..chars.iter().position(|c| *c == 0).unwrap_or(chars.len())])
}
struct Memory<T>(*mut T);
impl<T> Drop for Memory<T> {
    fn drop(&mut self) {
        unsafe {
            WlanFreeMemory(self.0 as _);
        }
    }
}
struct Native {
    handle: HANDLE,
    callback: *mut Sender<Action>,
}
impl Native {
    fn open() -> Result<Self, String> {
        unsafe {
            let mut handle = null_mut();
            let mut version = 0;
            if WlanOpenHandle(2, null(), &mut version, &mut handle) != 0 {
                return Err(
                    "无法打开 Windows WLAN 服务，请检查无线网卡及 WLAN AutoConfig 服务".into(),
                );
            }
            Ok(Self {
                handle,
                callback: null_mut(),
            })
        }
    }
    fn adapters(&self) -> Result<Vec<Adapter>, String> {
        unsafe {
            let mut list = null_mut();
            if WlanEnumInterfaces(self.handle, null(), &mut list) != 0 {
                return Err("无法枚举 Wi-Fi 网卡".into());
            }
            let _memory = Memory(list);
            let rows = std::slice::from_raw_parts(
                (*list).InterfaceInfo.as_ptr(),
                (*list).dwNumberOfItems as usize,
            );
            Ok(rows
                .iter()
                .map(|i| Adapter {
                    id: id(&i.InterfaceGuid),
                    name: string(&i.strInterfaceDescription),
                    guid: i.InterfaceGuid,
                })
                .collect())
        }
    }
    fn observe(&mut self, tx: Sender<Action>) -> Result<(), String> {
        unsafe {
            unsafe extern "system" fn changed(
                data: *mut L2_NOTIFICATION_DATA,
                context: *mut std::ffi::c_void,
            ) {
                if let Some(data) = data.as_ref() {
                    if data.NotificationSource == WLAN_NOTIFICATION_SOURCE_ACM {
                        let tx = &*(context as *const Sender<Action>);
                        let _ = tx.send(Action::Notify(data.InterfaceGuid, data.NotificationCode));
                    }
                }
            }
            self.callback = Box::into_raw(Box::new(tx));
            if WlanRegisterNotification(
                self.handle,
                WLAN_NOTIFICATION_SOURCE_ACM,
                1,
                Some(changed),
                self.callback as _,
                null(),
                null_mut(),
            ) != 0
            {
                drop(Box::from_raw(self.callback));
                self.callback = null_mut();
                return Err("无法订阅 Wi-Fi 变化通知，请检查系统定位权限".into());
            }
            Ok(())
        }
    }
    fn connected(&self, guid: &GUID) -> bool {
        unsafe {
            let mut size = 0;
            let mut data = null_mut();
            if WlanQueryInterface(
                self.handle,
                guid,
                wlan_intf_opcode_current_connection,
                null(),
                &mut size,
                &mut data,
                null_mut(),
            ) != 0
            {
                return false;
            }
            let _memory = Memory(data);
            if size < std::mem::size_of::<WLAN_CONNECTION_ATTRIBUTES>() as u32 {
                return false;
            }
            let current = &*(data as *const WLAN_CONNECTION_ATTRIBUTES);
            let ssid = &current.wlanAssociationAttributes.dot11Ssid;
            current.isState == wlan_interface_state_connected
                && ssid.uSSIDLength as usize == SSID.len()
                && &ssid.ucSSID[..SSID.len()] == SSID
        }
    }
    fn scan(&self, guid: &GUID) -> Result<(), String> {
        unsafe {
            let mut target = DOT11_SSID {
                uSSIDLength: SSID.len() as u32,
                ..DOT11_SSID::default()
            };
            target.ucSSID[..SSID.len()].copy_from_slice(SSID);
            if WlanScan(self.handle, guid, &target, null(), null()) != 0 {
                return Err("Wi-Fi 扫描失败，请检查无线开关和系统定位权限".into());
            }
            Ok(())
        }
    }
    fn target(&self, guid: &GUID) -> Result<Option<WLAN_AVAILABLE_NETWORK>, String> {
        unsafe {
            let mut list = null_mut();
            let result = WlanGetAvailableNetworkList(self.handle, guid, 0, null(), &mut list);
            if result != 0 {
                return Err(if result == 5 {
                    "Wi-Fi 自动连接需要系统定位权限，请在 Windows 设置中允许"
                } else {
                    "无法读取 Wi-Fi 扫描结果"
                }
                .into());
            }
            let _memory = Memory(list);
            let rows = std::slice::from_raw_parts(
                (*list).Network.as_ptr(),
                (*list).dwNumberOfItems as usize,
            );
            Ok(rows
                .iter()
                .filter(|n| {
                    n.dot11Ssid.uSSIDLength as usize == SSID.len()
                        && &n.dot11Ssid.ucSSID[..SSID.len()] == SSID
                        && n.bNetworkConnectable != 0
                        && n.dot11BssType == dot11_BSS_type_infrastructure
                })
                .max_by_key(|n| n.wlanSignalQuality)
                .copied())
        }
    }
    fn connect(&self, guid: &GUID, network: &WLAN_AVAILABLE_NETWORK) -> Result<(), String> {
        unsafe {
            let has_profile = network.dwFlags & WLAN_AVAILABLE_NETWORK_HAS_PROFILE != 0
                && network.strProfileName[0] != 0;
            if network.bSecurityEnabled != 0 && !has_profile {
                return Err(
                    "BUAA-WiFi 需要无线凭据，请先在系统中连接；校园网密码不会用于 Wi-Fi".into(),
                );
            }
            let mut ssid = network.dot11Ssid;
            let params = WLAN_CONNECTION_PARAMETERS {
                wlanConnectionMode: if has_profile {
                    wlan_connection_mode_profile
                } else {
                    wlan_connection_mode_discovery_unsecure
                },
                strProfile: if has_profile {
                    network.strProfileName.as_ptr()
                } else {
                    null()
                },
                pDot11Ssid: &mut ssid,
                pDesiredBssidList: null_mut(),
                dot11BssType: dot11_BSS_type_infrastructure,
                dwFlags: 0,
            };
            if WlanConnect(self.handle, guid, &params, null()) != 0 {
                return Err("无法连接 BUAA-WiFi，请检查无线开关和系统网络配置".into());
            }
            Ok(())
        }
    }
}
impl Drop for Native {
    fn drop(&mut self) {
        unsafe {
            if !self.callback.is_null() {
                WlanRegisterNotification(
                    self.handle,
                    WLAN_NOTIFICATION_SOURCE_NONE,
                    1,
                    None,
                    null(),
                    null(),
                    null_mut(),
                );
            }
            WlanCloseHandle(self.handle, null());
            if !self.callback.is_null() {
                drop(Box::from_raw(self.callback));
            }
        }
    }
}
pub fn adapters() -> Result<Vec<Adapter>, String> {
    Native::open()?.adapters()
}
enum Action {
    Refresh,
    Notify(GUID, u32),
    Quit,
}
struct State {
    enabled: Mutex<HashSet<String>>,
    active: AtomicBool,
    sleeping: AtomicBool,
}
impl State {
    fn allows(&self, id: &str) -> bool {
        self.active.load(Ordering::SeqCst)
            && !self.sleeping.load(Ordering::SeqCst)
            && self.enabled.lock().unwrap().contains(id)
    }
}
pub struct Wifi {
    tx: Sender<Action>,
    pub messages: Receiver<String>,
    state: Arc<State>,
    worker: Option<JoinHandle<()>>,
}
impl Wifi {
    pub fn new(window: HWND, message: u32) -> Self {
        let (tx, rx) = mpsc::channel();
        let (out, messages) = mpsc::channel();
        let state = Arc::new(State {
            enabled: Mutex::new(HashSet::new()),
            active: AtomicBool::new(false),
            sleeping: AtomicBool::new(false),
        });
        let worker_state = state.clone();
        let callbacks = tx.clone();
        let window = window as usize;
        let worker = std::thread::spawn(move || {
            let emit = |text: String| {
                if out.send(text).is_ok() {
                    unsafe {
                        PostMessageW(window as HWND, message, 0, 0);
                    }
                }
            };
            let mut native = None;
            let mut attempts: HashMap<String, Instant> = HashMap::new();
            let mut scans: HashMap<String, Instant> = HashMap::new();
            let mut last_error = String::new();
            while let Ok(action) = rx.recv() {
                if matches!(action, Action::Quit) {
                    break;
                }
                if !worker_state.active.load(Ordering::SeqCst)
                    || worker_state.sleeping.load(Ordering::SeqCst)
                {
                    native = None;
                    attempts.clear();
                    scans.clear();
                    continue;
                }
                if worker_state.enabled.lock().unwrap().is_empty() {
                    native = None;
                    attempts.clear();
                    scans.clear();
                    continue;
                }
                if native.is_none() {
                    match Native::open().and_then(|mut n| {
                        n.observe(callbacks.clone())?;
                        Ok(n)
                    }) {
                        Ok(n) => native = Some(n),
                        Err(e) => {
                            if last_error != e {
                                emit(e.clone());
                                last_error = e;
                            }
                            continue;
                        }
                    }
                }
                let n = native.as_ref().unwrap();
                let rows = match n.adapters() {
                    Ok(rows) => rows,
                    Err(e) => {
                        emit(e);
                        continue;
                    }
                };
                for adapter in rows {
                    if !worker_state.allows(&adapter.id) || n.connected(&adapter.guid) {
                        continue;
                    }
                    let active_scan = match &action {
                        Action::Refresh => true,
                        Action::Notify(guid, code) if id(guid) == adapter.id => {
                            if *code as i32 == wlan_notification_acm_connection_attempt_fail {
                                if attempts
                                    .get(&adapter.id)
                                    .is_some_and(|t| t.elapsed() < Duration::from_secs(30))
                                {
                                    emit(format!(
                                        "{}：BUAA-WiFi 自动连接失败，等待后续扫描结果",
                                        adapter.name
                                    ));
                                }
                                continue;
                            }
                            if ![
                                wlan_notification_acm_scan_complete,
                                wlan_notification_acm_scan_list_refresh,
                                wlan_notification_acm_network_available,
                                wlan_notification_acm_disconnected,
                                wlan_notification_acm_interface_arrival,
                                wlan_notification_acm_power_setting_change,
                            ]
                            .contains(&(*code as i32))
                            {
                                continue;
                            }
                            *code as i32 == wlan_notification_acm_disconnected
                                || *code as i32 == wlan_notification_acm_interface_arrival
                                || *code as i32 == wlan_notification_acm_power_setting_change
                        }
                        _ => continue,
                    };
                    if active_scan
                        && scans
                            .get(&adapter.id)
                            .is_none_or(|t| t.elapsed() >= Duration::from_secs(30))
                    {
                        scans.insert(adapter.id.clone(), Instant::now());
                        if let Err(e) = n.scan(&adapter.guid) {
                            if last_error != e {
                                emit(e.clone());
                                last_error = e;
                            }
                        }
                    }
                    if attempts
                        .get(&adapter.id)
                        .is_some_and(|t| t.elapsed() < Duration::from_secs(30))
                    {
                        continue;
                    }
                    match n.target(&adapter.guid) {
                        Ok(Some(network)) if worker_state.allows(&adapter.id) => {
                            attempts.insert(adapter.id.clone(), Instant::now());
                            match n.connect(&adapter.guid, &network) {
                                Ok(()) => {
                                    last_error.clear();
                                    emit(format!(
                                        "{}：发现 BUAA-WiFi，已请求自动连接",
                                        adapter.name
                                    ));
                                }
                                Err(e) => {
                                    if last_error != e {
                                        emit(e.clone());
                                        last_error = e;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            if last_error != e {
                                emit(e.clone());
                                last_error = e;
                            }
                        }
                        _ => {}
                    }
                }
            }
        });
        Self {
            tx,
            messages,
            state,
            worker: Some(worker),
        }
    }
    pub fn configure(&self, enabled: HashSet<String>, active: bool) {
        *self.state.enabled.lock().unwrap() = enabled;
        self.state.active.store(active, Ordering::SeqCst);
        let _ = self.tx.send(Action::Refresh);
    }
    pub fn refresh(&self) {
        let _ = self.tx.send(Action::Refresh);
    }
    pub fn suspend(&self) {
        self.state.sleeping.store(true, Ordering::SeqCst);
        self.refresh();
    }
    pub fn resume(&self) {
        self.state.sleeping.store(false, Ordering::SeqCst);
        self.refresh();
    }
}
impl Drop for Wifi {
    fn drop(&mut self) {
        self.state.active.store(false, Ordering::SeqCst);
        let _ = self.tx.send(Action::Quit);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
