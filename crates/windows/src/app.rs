use buaa_core::{
    engine::{Handle, Mode},
    Config, DashboardSnapshot, Event,
};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    ptr::{null, null_mut},
    sync::mpsc::{self, Receiver},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    Security::Cryptography::*,
    System::{
        LibraryLoader::*,
        SystemServices::{SS_CENTERIMAGE, SS_ICON},
        Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime},
    },
    UI::{
        Controls::*,
        Input::KeyboardAndMouse::{EnableWindow, SetFocus},
        Shell::*,
        WindowsAndMessaging::*,
    },
};
const EVENTS: u32 = WM_APP + 1;
const TRAY: u32 = WM_APP + 2;
const WIFI_EVENTS: u32 = WM_APP + 3;
const WIFI_ADAPTER: usize = 30;
const WIFI_AUTO: usize = 31;
const PAGES: usize = 32;
const ADVANCED: usize = 33;
const HISTORY: usize = 34;
const SETTINGS_START: usize = 35;
const AUTOSTART: usize = 36;
const START: usize = 10;
const STOP: usize = 11;
const SCAN: usize = 12;
const HIDE: usize = 13;
const SHOW: usize = 14;
const QUIT: usize = 15;
const REMEMBER: usize = 16;
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
struct App {
    window: HWND,
    username: HWND,
    password: HWND,
    interval: HWND,
    gateway: HWND,
    interfaces: HWND,
    remember: HWND,
    start: HWND,
    stop: HWND,
    scan: HWND,
    status: HWND,
    history: HWND,
    tabs: HWND,
    page_controls: RefCell<Vec<(HWND, usize)>>,
    building_page: Cell<Option<usize>>,
    building_advanced: Cell<bool>,
    advanced_controls: RefCell<Vec<HWND>>,
    settings_positions: RefCell<Vec<(HWND, i32, i32, i32, i32)>>,
    settings_pane: HWND,
    settings_scroll: i32,
    active_page: usize,
    advanced_expanded: bool,
    settings_start: HWND,
    settings_hint: HWND,
    startup: HWND,
    history_button: HWND,
    dashboard_values: [HWND; 6],
    state_title: HWND,
    state_icon: HWND,
    header_icon: HICON,
    title_font: HFONT,
    heading_font: HFONT,
    dpi: i32,
    scroll: i32,
    history_expanded: bool,
    bindings: Vec<Option<String>>,
    lines: Vec<String>,
    handle: Option<Handle>,
    receiver: Option<Receiver<Event>>,
    closing: bool,
    scanning: bool,
    tray: NOTIFYICONDATAW,
    font: HFONT,
    power: windows_sys::Win32::System::Power::HPOWERNOTIFY,
    wifi_adapter: HWND,
    wifi_auto: HWND,
    wifi_options: Vec<crate::wifi::Adapter>,
    wifi_enabled: std::collections::HashSet<String>,
    wifi: Option<crate::wifi::Wifi>,
}
impl App {
    fn new() -> Self {
        Self {
            window: null_mut(),
            username: null_mut(),
            password: null_mut(),
            interval: null_mut(),
            gateway: null_mut(),
            interfaces: null_mut(),
            remember: null_mut(),
            start: null_mut(),
            stop: null_mut(),
            scan: null_mut(),
            status: null_mut(),
            history: null_mut(),
            tabs: null_mut(),
            page_controls: RefCell::new(Vec::new()),
            building_page: Cell::new(None),
            building_advanced: Cell::new(false),
            advanced_controls: RefCell::new(Vec::new()),
            settings_positions: RefCell::new(Vec::new()),
            settings_pane: null_mut(),
            settings_scroll: 0,
            active_page: 0,
            advanced_expanded: false,
            settings_start: null_mut(),
            settings_hint: null_mut(),
            startup: null_mut(),
            history_button: null_mut(),
            dashboard_values: [null_mut(); 6],
            state_title: null_mut(),
            state_icon: null_mut(),
            header_icon: null_mut(),
            title_font: null_mut(),
            heading_font: null_mut(),
            dpi: 96,
            scroll: 0,
            history_expanded: false,
            bindings: vec![],
            lines: vec![],
            handle: None,
            receiver: None,
            closing: false,
            scanning: false,
            tray: unsafe { std::mem::zeroed() },
            font: null_mut(),
            power: 0,
            wifi_adapter: null_mut(),
            wifi_auto: null_mut(),
            wifi_options: vec![],
            wifi_enabled: std::collections::HashSet::new(),
            wifi: None,
        }
    }
    // Mirrors CreateWindowEx layout parameters for native controls.
    #[allow(clippy::too_many_arguments)]
    unsafe fn control(
        &self,
        class: &str,
        text: &str,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        id: usize,
    ) -> HWND {
        let hwnd = CreateWindowExW(
            0,
            wide(class).as_ptr(),
            wide(text).as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            self.px(x),
            self.px(y),
            self.px(width),
            self.px(height),
            if self.building_page.get() == Some(1) {
                self.settings_pane
            } else {
                self.window
            },
            id as HMENU,
            GetModuleHandleW(null()),
            null(),
        );
        if let Some(page) = self.building_page.get() {
            self.page_controls.borrow_mut().push((hwnd, page));
        }
        if self.building_page.get() == Some(1) {
            self.settings_positions
                .borrow_mut()
                .push((hwnd, x, y, width, height));
            if self.building_advanced.get() {
                self.advanced_controls.borrow_mut().push(hwnd);
            }
        }
        SendMessageW(hwnd, WM_SETFONT, self.font as usize, 1);
        SetWindowTheme(hwnd, wide("Explorer").as_ptr(), null());
        hwnd
    }
    fn px(&self, value: i32) -> i32 {
        (value * self.dpi + 48) / 96
    }
    unsafe fn label(&self, text: &str, x: i32, y: i32, width: i32) -> HWND {
        self.control("STATIC", text, 0, x, y, width, 22, 0)
    }
    unsafe fn build(&mut self) {
        self.dpi = system_dpi();
        let mut metrics: NONCLIENTMETRICSW = std::mem::zeroed();
        metrics.cbSize = std::mem::size_of::<NONCLIENTMETRICSW>() as u32;
        if SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            &mut metrics as *mut _ as _,
            0,
        ) != 0
        {
            self.font = CreateFontIndirectW(&metrics.lfMessageFont);
            let mut heading = metrics.lfMessageFont;
            heading.lfWeight = FW_SEMIBOLD as i32;
            self.heading_font = CreateFontIndirectW(&heading);
            heading.lfHeight = -self.px(24);
            self.title_font = CreateFontIndirectW(&heading);
        } else {
            self.font = CreateFontW(
                -self.px(13),
                0,
                0,
                0,
                FW_NORMAL as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                0,
                0,
                0,
                0,
                wide("Segoe UI").as_ptr(),
            );
            self.heading_font = CreateFontW(
                -self.px(13),
                0,
                0,
                0,
                FW_SEMIBOLD as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                0,
                0,
                0,
                0,
                wide("Segoe UI").as_ptr(),
            );
            self.title_font = CreateFontW(
                -self.px(24),
                0,
                0,
                0,
                FW_SEMIBOLD as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                0,
                0,
                0,
                0,
                wide("Segoe UI").as_ptr(),
            );
        }
        let mut stock: SHSTOCKICONINFO = std::mem::zeroed();
        stock.cbSize = std::mem::size_of_val(&stock) as u32;
        if SHGetStockIconInfo(SIID_MYNETWORK, SHGSI_ICON | SHGSI_LARGEICON, &mut stock) >= 0 {
            self.header_icon = stock.hIcon;
        }
        self.tabs = self.control(
            "SysTabControl32",
            "",
            WS_TABSTOP | WS_CLIPSIBLINGS,
            20,
            20,
            640,
            560,
            PAGES,
        );
        for (index, title) in ["连接仪表盘", "设置"].iter().enumerate() {
            let mut title = wide(title);
            let item = TCITEMW {
                mask: TCIF_TEXT,
                pszText: title.as_mut_ptr(),
                ..std::mem::zeroed()
            };
            SendMessageW(
                self.tabs,
                TCM_INSERTITEMW,
                index,
                &item as *const _ as isize,
            );
        }
        self.settings_pane = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            wide("BUAALoginSettingsPane").as_ptr(),
            wide("").as_ptr(),
            WS_CHILD | WS_VSCROLL | WS_CLIPCHILDREN,
            self.px(40),
            self.px(68),
            self.px(600),
            self.px(434),
            self.window,
            null_mut(),
            GetModuleHandleW(null()),
            self as *mut _ as _,
        );
        self.building_page.set(Some(0));
        self.state_icon = self.control("STATIC", "", SS_ICON | SS_CENTERIMAGE, 48, 78, 48, 48, 0);
        self.state_title = self.control("STATIC", "北航校园网", 0, 118, 72, 516, 36, 0);
        SendMessageW(self.state_title, WM_SETFONT, self.title_font as usize, 1);
        self.status = self.control(
            "STATIC",
            "先在设置中填写账号，再开始自动重连。",
            0,
            118,
            112,
            516,
            64,
            0,
        );
        self.update_state("idle");
        for (index, title) in [
            "当前接口",
            "本地 IP",
            "校园网 IP",
            "上次检查",
            "检查周期",
            "下次重试",
        ]
        .iter()
        .enumerate()
        {
            let x = 48 + (index as i32 % 2) * 300;
            let y = 196 + (index as i32 / 2) * 56;
            self.label(title, x, y, 280);
            self.dashboard_values[index] = self.label("—", x, y + 24, 280);
            SendMessageW(
                self.dashboard_values[index],
                WM_SETFONT,
                self.heading_font as usize,
                1,
            );
        }
        self.label("最近活动", 48, 368, 400);
        self.history_button = self.control(
            "BUTTON",
            "查看完整记录",
            WS_TABSTOP,
            484,
            362,
            150,
            28,
            HISTORY,
        );
        self.history = self.control(
            "EDIT",
            "暂无活动记录",
            WS_BORDER
                | ES_MULTILINE as u32
                | ES_READONLY as u32
                | ES_AUTOVSCROLL as u32
                | WS_VSCROLL
                | WS_TABSTOP,
            48,
            400,
            586,
            100,
            0,
        );
        self.start = self.control(
            "BUTTON",
            "设置账号",
            BS_DEFPUSHBUTTON as u32 | WS_TABSTOP,
            48,
            530,
            180,
            32,
            START,
        );
        self.stop = self.control("BUTTON", "停止", WS_TABSTOP, 242, 530, 86, 32, STOP);
        EnableWindow(self.stop, 0);
        self.control("BUTTON", "收起至托盘", WS_TABSTOP, 482, 530, 152, 32, HIDE);

        self.building_page.set(Some(1));
        let title = self.label("账号", 12, 4, 550);
        SendMessageW(title, WM_SETFONT, self.heading_font as usize, 1);
        self.label("用于登录校园网，凭据由 Windows 加密保存。", 12, 34, 550);
        self.label("校园网账号", 12, 76, 550);
        self.username = self.control(
            "EDIT",
            "",
            WS_BORDER | ES_AUTOHSCROLL as u32 | WS_TABSTOP,
            12,
            102,
            550,
            30,
            20,
        );
        SendMessageW(
            self.username,
            EM_SETCUEBANNER,
            0,
            wide("学号 / 工号").as_ptr() as isize,
        );
        self.label("密码", 12, 152, 550);
        self.password = self.control(
            "EDIT",
            "",
            WS_BORDER | ES_PASSWORD as u32 | ES_AUTOHSCROLL as u32 | WS_TABSTOP,
            12,
            178,
            550,
            30,
            21,
        );
        SendMessageW(
            self.password,
            EM_SETCUEBANNER,
            0,
            wide("请输入校园网密码").as_ptr() as isize,
        );
        self.remember = self.control(
            "BUTTON",
            "记住账号密码",
            BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
            12,
            224,
            550,
            24,
            REMEMBER,
        );
        self.startup = self.control(
            "BUTTON",
            "开机自启",
            BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
            12,
            266,
            550,
            24,
            AUTOSTART,
        );
        self.control(
            "STATIC",
            "登录系统后启动；需记住账号密码，保存完整配置后自动连接。",
            0,
            12,
            300,
            550,
            36,
            0,
        );
        let title = self.label("连接", 12, 358, 550);
        SendMessageW(title, WM_SETFONT, self.heading_font as usize, 1);
        self.label("校园网接口", 12, 400, 550);
        self.interfaces = self.control(
            "COMBOBOX",
            "",
            CBS_DROPDOWNLIST as u32 | WS_VSCROLL | WS_TABSTOP,
            12,
            426,
            374,
            260,
            23,
        );
        SendMessageW(
            self.interfaces,
            CB_ADDSTRING,
            0,
            wide("自动选择").as_ptr() as isize,
        );
        SendMessageW(self.interfaces, CB_SETCURSEL, 0, 0);
        self.bindings = vec![None];
        self.scan = self.control("BUTTON", "检测接口", WS_TABSTOP, 402, 426, 160, 28, SCAN);
        self.label("检查间隔", 12, 476, 550);
        self.interval = self.control(
            "EDIT",
            "300",
            WS_BORDER | ES_NUMBER as u32 | WS_TABSTOP,
            12,
            502,
            100,
            28,
            22,
        );
        self.label("秒 · 最低 10 秒，失败时采用递增退避", 128, 508, 434);
        self.control(
            "BUTTON",
            "展开高级选项",
            WS_TABSTOP,
            12,
            556,
            170,
            28,
            ADVANCED,
        );
        self.building_advanced.set(true);
        self.label("网关地址", 12, 604, 550);
        self.gateway = self.control(
            "EDIT",
            "10.200.21.4",
            WS_BORDER | WS_TABSTOP,
            12,
            630,
            240,
            28,
            24,
        );
        self.label("通常无需修改", 268, 636, 294);
        self.label("Wi-Fi 自动连接", 12, 682, 550);
        self.wifi_adapter = self.control(
            "COMBOBOX",
            "",
            CBS_DROPDOWNLIST as u32 | WS_VSCROLL | WS_TABSTOP,
            12,
            708,
            550,
            260,
            WIFI_ADAPTER,
        );
        self.wifi_auto = self.control(
            "BUTTON",
            "发现 BUAA-WiFi 自动连接",
            BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
            12,
            760,
            550,
            24,
            WIFI_AUTO,
        );
        self.building_advanced.set(false);
        if let Ok(path) = wifi_path() {
            if let Ok(data) = std::fs::read(path) {
                self.wifi_enabled = serde_json::from_slice(&data).unwrap_or_default();
            }
        }
        self.wifi = Some(crate::wifi::Wifi::new(self.window, WIFI_EVENTS));
        self.refresh_wifi_options();
        self.building_page.set(None);
        self.settings_start = self.control(
            "BUTTON",
            "开始自动重连",
            WS_TABSTOP,
            448,
            530,
            186,
            32,
            SETTINGS_START,
        );
        self.settings_hint = self.label("开始时保存并应用设置。", 48, 530, 386);
        self.page_controls
            .borrow_mut()
            .extend([(self.settings_start, 1), (self.settings_hint, 1)]);
        self.update_settings_scroll();
        self.show_page(0);
        self.tray.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        self.tray.hWnd = self.window;
        self.tray.uID = 1;
        self.tray.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        self.tray.uCallbackMessage = TRAY;
        self.tray.hIcon = if self.header_icon.is_null() {
            LoadIconW(null_mut(), IDI_INFORMATION)
        } else {
            self.header_icon
        };
        self.tooltip("北航校园网 · 未启动");
        Shell_NotifyIconW(NIM_ADD, &self.tray);
        self.power = windows_sys::Win32::System::Power::RegisterSuspendResumeNotification(
            self.window,
            DEVICE_NOTIFY_WINDOW_HANDLE,
        );
        self.load_credentials();
        match crate::autostart::enabled() {
            Ok(enabled) => {
                SendMessageW(
                    self.startup,
                    BM_SETCHECK,
                    if enabled { BST_CHECKED } else { BST_UNCHECKED } as usize,
                    0,
                );
            }
            Err(e) => self.record(&e),
        }
    }
    unsafe fn update_state(&self, state: &str) {
        let (title, icon) = match state {
            "online" => ("校园网在线", IDI_INFORMATION),
            "retry" => ("等待自动重试", IDI_WARNING),
            "error" | "failed" => ("连接需要处理", IDI_WARNING),
            "suspended" => ("睡眠期间暂停", IDI_INFORMATION),
            "waiting_network" => ("等待网络", IDI_INFORMATION),
            "checking" => ("正在检查连接", IDI_INFORMATION),
            "interfaces" => ("接口检测完成", IDI_INFORMATION),
            "stopped" => ("已停止自动重连", IDI_INFORMATION),
            _ => ("北航校园网", IDI_INFORMATION),
        };
        SetWindowTextW(self.state_title, wide(title).as_ptr());
        SendMessageW(
            self.state_icon,
            STM_SETICON,
            LoadIconW(null_mut(), icon) as usize,
            0,
        );
    }
    unsafe fn show_page(&mut self, index: usize) {
        self.active_page = index;
        SendMessageW(self.tabs, TCM_SETCURSEL, index, 0);
        for &(control, page) in self.page_controls.borrow().iter() {
            let advanced = self.advanced_controls.borrow().contains(&control);
            ShowWindow(
                control,
                if page == index && (!advanced || self.advanced_expanded) {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
        }
        ShowWindow(
            self.settings_pane,
            if index == 1 { SW_SHOW } else { SW_HIDE },
        );
        if index == 0 {
            self.render_history();
        }
        SendMessageW(
            self.start,
            BM_SETSTYLE,
            if index == 0 {
                BS_DEFPUSHBUTTON
            } else {
                BS_PUSHBUTTON
            } as usize,
            1,
        );
        SendMessageW(
            self.settings_start,
            BM_SETSTYLE,
            if index == 1 {
                BS_DEFPUSHBUTTON
            } else {
                BS_PUSHBUTTON
            } as usize,
            1,
        );
        SetFocus(self.tabs);
        InvalidateRect(self.window, null(), 1);
    }
    unsafe fn toggle_history(&mut self) {
        self.history_expanded = !self.history_expanded;
        SetWindowTextW(
            self.history_button,
            wide(if self.history_expanded {
                "收起完整记录"
            } else {
                "查看完整记录"
            })
            .as_ptr(),
        );
        self.render_history();
    }
    unsafe fn toggle_advanced(&mut self) {
        self.advanced_expanded = !self.advanced_expanded;
        let button = GetDlgItem(self.settings_pane, ADVANCED as i32);
        SetWindowTextW(
            button,
            wide(if self.advanced_expanded {
                "收起高级选项"
            } else {
                "展开高级选项"
            })
            .as_ptr(),
        );
        for &control in self.advanced_controls.borrow().iter() {
            ShowWindow(
                control,
                if self.advanced_expanded && self.active_page == 1 {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
        }
        self.update_settings_scroll();
    }
    unsafe fn update_settings_scroll(&mut self) {
        if self.settings_pane.is_null() {
            return;
        }
        let mut area: RECT = std::mem::zeroed();
        GetClientRect(self.settings_pane, &mut area);
        let mut info = SCROLLINFO {
            cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
            fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
            nMin: 0,
            nMax: self.px(if self.advanced_expanded { 810 } else { 592 }) - 1,
            nPage: (area.bottom - area.top).max(0) as u32,
            nPos: self.settings_scroll,
            nTrackPos: 0,
        };
        SetScrollInfo(self.settings_pane, SB_VERT, &info, 1);
        info.fMask = SIF_POS;
        GetScrollInfo(self.settings_pane, SB_VERT, &mut info);
        self.settings_scroll = info.nPos;
        for &(control, x, y, width, height) in self.settings_positions.borrow().iter() {
            MoveWindow(
                control,
                self.px(x),
                self.px(y) - self.settings_scroll,
                self.px(width),
                self.px(height),
                1,
            );
        }
    }
    unsafe fn ensure_setting_visible(&mut self, field: HWND) {
        let position = self
            .settings_positions
            .borrow()
            .iter()
            .find(|p| p.0 == field)
            .copied();
        let Some((_, _, y, _, height)) = position else {
            return;
        };
        let mut area: RECT = std::mem::zeroed();
        GetClientRect(self.settings_pane, &mut area);
        let top = self.px(y);
        let bottom = top + self.px(height.min(32));
        if top < self.settings_scroll {
            self.settings_scroll = (top - self.px(12)).max(0);
        } else if bottom > self.settings_scroll + area.bottom {
            self.settings_scroll = bottom - area.bottom + self.px(12);
        } else {
            return;
        }
        self.update_settings_scroll();
    }
    unsafe fn focus_setting(&mut self, field: HWND) {
        self.show_page(1);
        if field == self.gateway && !self.advanced_expanded {
            self.toggle_advanced();
        }
        let y = self
            .settings_positions
            .borrow()
            .iter()
            .find(|c| c.0 == field)
            .map(|c| c.2)
            .unwrap_or(0);
        self.settings_scroll = self.px((y - 30).max(0));
        self.update_settings_scroll();
        SetFocus(field);
        SendMessageW(field, EM_SETSEL, 0, -1);
    }
    unsafe fn complete_configuration(&self) -> Option<Config> {
        let config = Config {
            username: text(self.username).trim().into(),
            password: text(self.password).into(),
            interval: text(self.interval).parse().ok()?,
            gateway_ip: text(self.gateway).trim().parse().ok()?,
            interface: self
                .bindings
                .get(SendMessageW(self.interfaces, CB_GETCURSEL, 0, 0) as usize)
                .cloned()
                .flatten(),
            ..Config::default()
        };
        config.validate(true).ok()?;
        Some(config)
    }
    unsafe fn startup_changed(&mut self) {
        let enabled = SendMessageW(self.startup, BM_GETCHECK, 0, 0) == BST_CHECKED as isize;
        let result = (|| -> Result<(), String> {
            if enabled && self.handle.is_none() {
                if let Some(config) = self.complete_configuration() {
                    self.save_credentials(&config)?;
                }
            }
            crate::autostart::set_enabled(enabled)
        })();
        if let Err(e) = result {
            SendMessageW(
                self.startup,
                BM_SETCHECK,
                if crate::autostart::enabled().unwrap_or(!enabled) {
                    BST_CHECKED
                } else {
                    BST_UNCHECKED
                } as usize,
                0,
            );
            MessageBoxW(
                self.window,
                wide(&e).as_ptr(),
                wide("开机自启").as_ptr(),
                MB_OK | MB_ICONINFORMATION,
            );
        }
    }
    unsafe fn restore_binding(&mut self, binding: Option<&str>) {
        let index = if let Some(binding) = binding {
            if let Some(index) = self
                .bindings
                .iter()
                .position(|b| b.as_deref() == Some(binding))
            {
                index
            } else {
                SendMessageW(
                    self.interfaces,
                    CB_ADDSTRING,
                    0,
                    wide(&format!("指定接口 · {}", binding)).as_ptr() as isize,
                );
                self.bindings.push(Some(binding.into()));
                self.bindings.len() - 1
            }
        } else {
            0
        };
        SendMessageW(self.interfaces, CB_SETCURSEL, index, 0);
    }
    unsafe fn update_start_title(&self) {
        if self.start.is_null() {
            return;
        }
        let configured = self.handle.is_some()
            || (!text(self.username).trim().is_empty() && !text(self.password).is_empty());
        SetWindowTextW(
            self.start,
            wide(if configured {
                "开始自动重连"
            } else {
                "设置账号"
            })
            .as_ptr(),
        );
        if !self.dashboard_values[4].is_null() && self.handle.is_none() {
            SetWindowTextW(
                self.dashboard_values[4],
                wide(&format!("{} 秒", text(self.interval))).as_ptr(),
            );
        }
    }
    unsafe fn dashboard_action(&mut self) {
        if text(self.username).trim().is_empty() || text(self.password).is_empty() {
            self.focus_setting(self.username);
        } else {
            self.start_worker(false);
        }
    }
    unsafe fn apply_dashboard(&mut self, snapshot: &DashboardSnapshot, message: &str) {
        self.update_state(&snapshot.phase);
        let values = [
            snapshot.interface.clone().unwrap_or_else(|| "—".into()),
            snapshot
                .local_ip
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "—".into()),
            snapshot.campus_ip.clone().unwrap_or_else(|| "—".into()),
            local_time(snapshot.last_check_at),
            format!("{} 秒", snapshot.interval),
            local_time(snapshot.retry_at),
        ];
        for (control, value) in self.dashboard_values.iter().zip(values) {
            SetWindowTextW(*control, wide(&value).as_ptr());
        }
        SetWindowTextW(self.status, wide(message).as_ptr());
        self.tooltip(message);
        Shell_NotifyIconW(NIM_MODIFY, &self.tray);
    }
    unsafe fn update_scroll(&mut self) {
        let mut area: RECT = std::mem::zeroed();
        GetClientRect(self.window, &mut area);
        let mut info = SCROLLINFO {
            cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
            fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
            nMin: 0,
            nMax: self.px(600) - 1,
            nPage: (area.bottom - area.top).max(0) as u32,
            nPos: self.scroll,
            nTrackPos: 0,
        };
        SetScrollInfo(self.window, SB_VERT, &info, 1);
        info.fMask = SIF_POS;
        GetScrollInfo(self.window, SB_VERT, &mut info);
        if info.nPos != self.scroll {
            self.scroll_to(info.nPos);
        }
    }
    unsafe fn scroll_to(&mut self, target: i32) {
        let old = self.scroll;
        let mut info = SCROLLINFO {
            cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
            fMask: SIF_POS,
            nPos: target,
            ..std::mem::zeroed()
        };
        SetScrollInfo(self.window, SB_VERT, &info, 1);
        GetScrollInfo(self.window, SB_VERT, &mut info);
        self.scroll = info.nPos;
        ScrollWindowEx(
            self.window,
            0,
            old - self.scroll,
            null(),
            null(),
            null_mut(),
            null_mut(),
            SW_SCROLLCHILDREN | SW_INVALIDATE | SW_ERASE,
        );
    }
    unsafe fn render_history(&self) {
        let start = if self.history_expanded {
            0
        } else {
            self.lines.len().saturating_sub(5)
        };
        let text = if self.lines.is_empty() {
            "暂无活动记录".into()
        } else {
            self.lines[start..].join("\r\n")
        };
        SetWindowTextW(self.history, wide(&text).as_ptr());
        SendMessageW(self.history, EM_SETSEL, text.encode_utf16().count(), -1);
        SendMessageW(self.history, EM_SCROLLCARET, 0, 0);
    }
    unsafe fn tooltip(&mut self, text: &str) {
        self.tray.szTip.fill(0);
        for (out, input) in self
            .tray
            .szTip
            .iter_mut()
            .take(127)
            .zip(text.encode_utf16())
        {
            *out = input;
        }
    }
    unsafe fn record(&mut self, text: &str) {
        if self.handle.is_none() || self.scanning {
            SetWindowTextW(self.status, wide(text).as_ptr());
            self.tooltip(text);
            Shell_NotifyIconW(NIM_MODIFY, &self.tray);
        }
        self.lines
            .push(format!("{}  {}", local_time(Some(unix_now())), text));
        if self.lines.len() > 120 {
            self.lines.remove(0);
        }
        if self.active_page == 0 {
            self.render_history();
        }
    }
    unsafe fn busy(&self, busy: bool) {
        for c in [
            self.username,
            self.password,
            self.interval,
            self.gateway,
            self.interfaces,
            self.remember,
            self.start,
            self.scan,
        ] {
            EnableWindow(c, (!busy) as i32);
        }
        EnableWindow(self.stop, (busy && !self.scanning) as i32);
        EnableWindow(self.settings_start, (!busy) as i32);
        SetWindowTextW(
            self.settings_hint,
            wide(if busy {
                "修改配置，请先到仪表盘停止。"
            } else {
                "开始时保存并应用设置。"
            })
            .as_ptr(),
        );
        self.update_start_title();
    }
    unsafe fn start_worker(&mut self, scan: bool) {
        if self.handle.is_some() {
            return;
        }
        if scan {
            self.refresh_wifi_options();
            if let Some(wifi) = &self.wifi {
                wifi.refresh();
            }
        }
        let seconds = match text(self.interval).parse::<u64>() {
            Ok(v) => v,
            Err(_) => {
                self.focus_setting(self.interval);
                MessageBoxW(
                    self.window,
                    wide("检查间隔须为 10–86400 的整数").as_ptr(),
                    wide("请检查设置").as_ptr(),
                    MB_OK | MB_ICONINFORMATION,
                );
                self.record("检查间隔无效");
                return;
            }
        };
        let gateway_ip = match text(self.gateway).parse() {
            Ok(v) => v,
            Err(_) => {
                self.focus_setting(self.gateway);
                MessageBoxW(
                    self.window,
                    wide("网关必须是 IPv4 地址").as_ptr(),
                    wide("请检查设置").as_ptr(),
                    MB_OK | MB_ICONINFORMATION,
                );
                self.record("网关必须是 IPv4 地址");
                return;
            }
        };
        let selected = SendMessageW(self.interfaces, CB_GETCURSEL, 0, 0) as usize;
        let config = Config {
            username: if scan {
                String::new()
            } else {
                text(self.username).trim().into()
            },
            password: if scan {
                String::new().into()
            } else {
                text(self.password).into()
            },
            interval: seconds,
            gateway_ip,
            interface: if scan {
                None
            } else {
                self.bindings.get(selected).cloned().flatten()
            },
            ..Config::default()
        };
        if let Err(e) = config.validate(!scan) {
            self.focus_setting(if !scan && config.username.is_empty() {
                self.username
            } else if !scan && config.password.is_empty() {
                self.password
            } else {
                self.interval
            });
            MessageBoxW(
                self.window,
                wide(&e).as_ptr(),
                wide("请检查设置").as_ptr(),
                MB_OK | MB_ICONINFORMATION,
            );
            self.record(&e);
            return;
        }
        if !scan {
            if let Err(e) = self.save_credentials(&config) {
                self.record(&e);
                return;
            }
        }
        let (tx, rx) = mpsc::channel();
        let window = self.window as usize;
        match Handle::spawn(
            config,
            if scan { Mode::Inspect } else { Mode::Monitor },
            move |event| {
                if tx.send(event).is_ok() {
                    PostMessageW(window as HWND, EVENTS, 0, 0);
                }
            },
        ) {
            Ok(handle) => {
                self.handle = Some(handle);
                self.receiver = Some(rx);
                self.scanning = scan;
                self.update_state("checking");
                if let Some(wifi) = &self.wifi {
                    wifi.configure(self.wifi_enabled.clone(), !scan);
                }
                self.busy(true);
                if !scan {
                    self.show_page(0);
                    SetWindowTextW(self.password, wide("").as_ptr());
                }
                self.record(if scan {
                    "正在检测校园网接口…"
                } else {
                    "正在检查校园网连接…"
                });
            }
            Err(e) => self.record(&e),
        }
    }
    unsafe fn receive(&mut self) {
        let events: Vec<_> = self
            .receiver
            .as_ref()
            .map(|r| r.try_iter().collect())
            .unwrap_or_default();
        for event in events {
            if event.state == "dashboard" {
                if let Some(snapshot) = &event.dashboard {
                    self.apply_dashboard(snapshot, &event.message);
                }
                continue;
            }
            if (self.handle.is_none() || self.scanning)
                && event.state != "schedule"
                && (event.state != "stopped" || !self.scanning)
            {
                self.update_state(&event.state);
            }
            if event.state == "interfaces" {
                let old = self
                    .bindings
                    .get(SendMessageW(self.interfaces, CB_GETCURSEL, 0, 0) as usize)
                    .cloned()
                    .flatten();
                SendMessageW(self.interfaces, CB_RESETCONTENT, 0, 0);
                SendMessageW(
                    self.interfaces,
                    CB_ADDSTRING,
                    0,
                    wide("自动选择").as_ptr() as isize,
                );
                self.bindings = vec![None];
                let mut selected = 0;
                for row in event.records.unwrap_or_default() {
                    let index = self.bindings.len();
                    if old.as_ref() == Some(&row.binding) {
                        selected = index;
                    }
                    self.bindings.push(Some(row.binding));
                    SendMessageW(
                        self.interfaces,
                        CB_ADDSTRING,
                        0,
                        wide(&format!("{} · {}", row.label, row.message)).as_ptr() as isize,
                    );
                }
                if old.is_some() && selected == 0 {
                    self.restore_binding(old.as_deref());
                } else {
                    SendMessageW(self.interfaces, CB_SETCURSEL, selected, 0);
                }
            }
            if event.state == "stopped" {
                if let Some(wifi) = &self.wifi {
                    wifi.configure(self.wifi_enabled.clone(), false);
                }
                self.handle.take();
                self.receiver.take();
                self.busy(false);
                self.load_credentials();
                if self.closing {
                    DestroyWindow(self.window);
                    return;
                }
                if self.scanning {
                    self.scanning = false;
                    continue;
                }
            }
            if event.state != "schedule" {
                let mut message = event.message;
                if event.retry_at.is_some() {
                    message.push_str(&format!("（下次 {}）", local_time(event.retry_at)));
                }
                self.record(&message);
            }
        }
    }
    unsafe fn stop_worker(&mut self) {
        if let Some(wifi) = &self.wifi {
            wifi.configure(self.wifi_enabled.clone(), false);
        }
        if let Some(h) = &self.handle {
            h.signal.stop();
            EnableWindow(self.stop, 0);
            self.record("正在停止…");
        }
    }
    unsafe fn quit(&mut self) {
        self.closing = true;
        if self.handle.is_some() {
            self.stop_worker();
        } else {
            DestroyWindow(self.window);
        }
    }
    unsafe fn show(&self) {
        ShowWindow(self.window, SW_RESTORE);
        SetForegroundWindow(self.window);
    }
    unsafe fn tray_menu(&self) {
        let menu = CreatePopupMenu();
        AppendMenuW(menu, MF_STRING, SHOW, wide("显示窗口").as_ptr());
        AppendMenuW(menu, MF_STRING, QUIT, wide("退出").as_ptr());
        let mut point: POINT = std::mem::zeroed();
        GetCursorPos(&mut point);
        SetForegroundWindow(self.window);
        TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            self.window,
            null(),
        );
        DestroyMenu(menu);
    }
    unsafe fn refresh_wifi_options(&mut self) {
        let selected = self
            .wifi_options
            .get(SendMessageW(self.wifi_adapter, CB_GETCURSEL, 0, 0) as usize)
            .map(|a| a.id.clone());
        self.wifi_options = crate::wifi::adapters().unwrap_or_default();
        SendMessageW(self.wifi_adapter, CB_RESETCONTENT, 0, 0);
        let mut index = 0;
        for (i, adapter) in self.wifi_options.iter().enumerate() {
            SendMessageW(
                self.wifi_adapter,
                CB_ADDSTRING,
                0,
                wide(&format!("{} · {}", adapter.name, &adapter.id[..8])).as_ptr() as isize,
            );
            if selected.as_ref() == Some(&adapter.id) {
                index = i;
            }
        }
        if self.wifi_options.is_empty() {
            SendMessageW(
                self.wifi_adapter,
                CB_ADDSTRING,
                0,
                wide("未发现 Wi-Fi 网卡").as_ptr() as isize,
            );
        }
        SendMessageW(self.wifi_adapter, CB_SETCURSEL, index, 0);
        SendMessageW(self.wifi_adapter, CB_SETDROPPEDWIDTH, 400, 0);
        self.show_wifi_option();
    }
    unsafe fn show_wifi_option(&self) {
        let selected = self
            .wifi_options
            .get(SendMessageW(self.wifi_adapter, CB_GETCURSEL, 0, 0) as usize);
        EnableWindow(self.wifi_auto, selected.is_some() as i32);
        SendMessageW(
            self.wifi_auto,
            BM_SETCHECK,
            if selected.is_some_and(|a| self.wifi_enabled.contains(&a.id)) {
                BST_CHECKED
            } else {
                BST_UNCHECKED
            } as usize,
            0,
        );
    }
    unsafe fn wifi_option_changed(&mut self) {
        let Some(adapter) =
            self.wifi_options
                .get(SendMessageW(self.wifi_adapter, CB_GETCURSEL, 0, 0) as usize)
        else {
            return;
        };
        let old = self.wifi_enabled.clone();
        if SendMessageW(self.wifi_auto, BM_GETCHECK, 0, 0) == BST_CHECKED as isize {
            self.wifi_enabled.insert(adapter.id.clone());
        } else {
            self.wifi_enabled.remove(&adapter.id);
        }
        let result = wifi_path().and_then(|path| {
            std::fs::create_dir_all(path.parent().unwrap())
                .map_err(|_| "无法创建 Wi-Fi 配置目录")?;
            atomic_write(&path, &serde_json::to_vec(&self.wifi_enabled).unwrap())
        });
        if let Err(e) = result {
            self.wifi_enabled = old;
            self.show_wifi_option();
            self.record(&e);
            return;
        }
        if let Some(wifi) = &self.wifi {
            wifi.configure(
                self.wifi_enabled.clone(),
                self.handle.is_some() && !self.scanning,
            );
        }
    }
    unsafe fn save_credentials(&self, config: &Config) -> Result<(), String> {
        let path = credential_path()?;
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|_| "无法创建配置目录")?;
        let settings = serde_json::json!({"interval":config.interval,"gateway":config.gateway_ip.to_string(),"interface":config.interface});
        atomic_write(
            &path.with_extension("settings.json"),
            &serde_json::to_vec(&settings).unwrap(),
        )?;
        if SendMessageW(self.remember, BM_GETCHECK, 0, 0) == BST_CHECKED as isize {
            let mut bytes = serde_json::to_vec(&serde_json::json!({"username":config.username,"password":config.password.as_str()})).unwrap();
            let encrypted = protect(&mut bytes, false);
            bytes.fill(0);
            atomic_write(&path, &encrypted?)?;
        } else if path.exists() {
            std::fs::remove_file(path).map_err(|_| "无法删除凭据")?;
        }
        Ok(())
    }
    unsafe fn load_credentials(&mut self) {
        let Ok(path) = credential_path() else {
            return;
        };
        if let Ok(bytes) = std::fs::read(path.with_extension("settings.json")) {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                if let Some(n) = v["interval"].as_u64() {
                    SetWindowTextW(self.interval, wide(&n.to_string()).as_ptr());
                }
                if let Some(g) = v["gateway"].as_str() {
                    SetWindowTextW(self.gateway, wide(g).as_ptr());
                }
                self.restore_binding(v["interface"].as_str());
            }
        }
        if let Ok(mut bytes) = std::fs::read(&path) {
            match protect(&mut bytes, true) {
                Ok(mut plain) => {
                    if let Ok(mut v) = serde_json::from_slice::<serde_json::Value>(&plain) {
                        SetWindowTextW(
                            self.username,
                            wide(v["username"].as_str().unwrap_or("")).as_ptr(),
                        );
                        SetWindowTextW(
                            self.password,
                            wide(v["password"].as_str().unwrap_or("")).as_ptr(),
                        );
                        if let Some(s) = v["password"].as_str() {
                            let _ = s;
                        }
                        v["password"] = serde_json::Value::Null;
                        SendMessageW(self.remember, BM_SETCHECK, BST_CHECKED as usize, 0);
                    } else {
                        self.record("原生版凭据格式错误，请重新输入");
                    }
                    plain.fill(0);
                }
                Err(_) => self.record("原生版凭据无法解密，请重新输入"),
            }
        }
        self.update_start_title();
    }
}
unsafe fn text(hwnd: HWND) -> String {
    let mut chars = vec![0u16; GetWindowTextLengthW(hwnd) as usize + 1];
    let len = GetWindowTextW(hwnd, chars.as_mut_ptr(), chars.len() as i32);
    let text = String::from_utf16_lossy(&chars[..len as usize]);
    chars.fill(0);
    text
}
fn wifi_path() -> Result<PathBuf, String> {
    Ok(credential_path()?.with_file_name("wifi.json"))
}
fn credential_path() -> Result<PathBuf, String> {
    Ok(
        PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("用户目录不可用")?)
            .join("BUAALoginNative/v1/credentials.dpapi"),
    )
}
fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|_| "无法保存配置")?;
    unsafe {
        windows_sys::Win32::Storage::FileSystem::MoveFileExW(
            wide(tmp.to_str().ok_or("配置路径错误")?).as_ptr(),
            wide(path.to_str().ok_or("配置路径错误")?).as_ptr(),
            windows_sys::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING
                | windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH,
        )
    };
    // Verify the move rather than silently accepting a failed atomic replacement.
    if tmp.exists() {
        return Err("无法替换配置文件".into());
    }
    Ok(())
}
unsafe fn protect(bytes: &mut [u8], decrypt: bool) -> Result<Vec<u8>, String> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_mut_ptr(),
    };
    let mut output: CRYPT_INTEGER_BLOB = std::mem::zeroed();
    let result = if decrypt {
        CryptUnprotectData(
            &input,
            null_mut(),
            null(),
            null(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    } else {
        CryptProtectData(
            &input,
            wide("BUAALoginNative v1").as_ptr(),
            null(),
            null(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if result == 0 {
        return Err("无法处理当前用户加密凭据".into());
    }
    let slice = std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize);
    let out = slice.to_vec();
    slice.fill(0);
    LocalFree(output.pbData as _);
    Ok(out)
}
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn local_time(timestamp: Option<u64>) -> String {
    let Some(timestamp) = timestamp else {
        return "—".into();
    };
    unsafe {
        let ticks = (timestamp + 11_644_473_600) * 10_000_000;
        let filetime = FILETIME {
            dwLowDateTime: ticks as u32,
            dwHighDateTime: (ticks >> 32) as u32,
        };
        let mut utc: SYSTEMTIME = std::mem::zeroed();
        let mut local: SYSTEMTIME = std::mem::zeroed();
        if FileTimeToSystemTime(&filetime, &mut utc) != 0
            && SystemTimeToTzSpecificLocalTime(null(), &utc, &mut local) != 0
        {
            return format!(
                "{:02}:{:02}:{:02}",
                local.wHour, local.wMinute, local.wSecond
            );
        }
    }
    "—".into()
}
unsafe extern "system" fn settings_proc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if message == WM_NCCREATE {
        let create = &*(l as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    let app = (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App).as_mut();
    if let Some(app) = app {
        match message {
            WM_COMMAND => {
                let notification = (w >> 16) as u32;
                if notification == EN_SETFOCUS || notification == CBN_SETFOCUS {
                    app.ensure_setting_visible(l as HWND);
                }
                return SendMessageW(app.window, message, w, l);
            }
            WM_NOTIFY => return SendMessageW(app.window, message, w, l),
            WM_VSCROLL | WM_MOUSEWHEEL => {
                let mut info = SCROLLINFO {
                    cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                    fMask: SIF_ALL,
                    ..std::mem::zeroed()
                };
                GetScrollInfo(hwnd, SB_VERT, &mut info);
                app.settings_scroll = if message == WM_MOUSEWHEEL {
                    app.settings_scroll - ((w >> 16) as i16 as i32) * app.px(40) / 120
                } else {
                    match (w & 0xffff) as i32 {
                        SB_LINEUP => app.settings_scroll - app.px(28),
                        SB_LINEDOWN => app.settings_scroll + app.px(28),
                        SB_PAGEUP => app.settings_scroll - info.nPage as i32,
                        SB_PAGEDOWN => app.settings_scroll + info.nPage as i32,
                        SB_THUMBTRACK | SB_THUMBPOSITION => info.nTrackPos,
                        SB_TOP => 0,
                        SB_BOTTOM => info.nMax,
                        _ => app.settings_scroll,
                    }
                };
                app.update_settings_scroll();
                return 0;
            }
            _ => {}
        }
    }
    DefWindowProcW(hwnd, message, w, l)
}
unsafe extern "system" fn wndproc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if message == WM_NCCREATE {
        let create = &*(l as *const CREATESTRUCTW);
        let app = create.lpCreateParams as *mut App;
        (*app).window = hwnd;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, app as isize);
    }
    let app = (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App).as_mut();
    if let Some(app) = app {
        match message {
            WM_CREATE => {
                app.build();
                return 0;
            }
            WM_NOTIFY => {
                let notification = &*(l as *const NMHDR);
                if notification.hwndFrom == app.tabs && notification.code == TCN_SELCHANGE {
                    let index = SendMessageW(app.tabs, TCM_GETCURSEL, 0, 0) as usize;
                    app.show_page(index);
                    return 0;
                }
            }
            DM_GETDEFID => {
                let id = if app.active_page == 0 {
                    START
                } else {
                    SETTINGS_START
                };
                return ((DC_HASDEFID as usize) << 16 | id) as isize;
            }
            WM_COMMAND => {
                match w & 0xffff {
                    START => app.dashboard_action(),
                    SETTINGS_START => app.start_worker(false),
                    ADVANCED => app.toggle_advanced(),
                    HISTORY => app.toggle_history(),
                    AUTOSTART => app.startup_changed(),
                    20 | 21 | 22 if w >> 16 == EN_CHANGE as usize => app.update_start_title(),
                    STOP => app.stop_worker(),
                    SCAN => app.start_worker(true),
                    HIDE => {
                        ShowWindow(hwnd, SW_HIDE);
                    }
                    SHOW => app.show(),
                    QUIT => app.quit(),
                    WIFI_ADAPTER if w >> 16 == CBN_SELCHANGE as usize => app.show_wifi_option(),
                    WIFI_AUTO => app.wifi_option_changed(),
                    REMEMBER => {
                        if SendMessageW(app.remember, BM_GETCHECK, 0, 0) != BST_CHECKED as isize {
                            if let Ok(path) = credential_path() {
                                if path.exists() && std::fs::remove_file(path).is_err() {
                                    SendMessageW(
                                        app.remember,
                                        BM_SETCHECK,
                                        BST_CHECKED as usize,
                                        0,
                                    );
                                    app.record("无法删除已保存凭据");
                                }
                            }
                        }
                    }
                    _ => {}
                }
                return 0;
            }
            EVENTS => {
                app.receive();
                return 0;
            }
            WIFI_EVENTS => {
                let messages: Vec<_> = app
                    .wifi
                    .as_ref()
                    .map(|wifi| wifi.messages.try_iter().collect())
                    .unwrap_or_default();
                for message in messages {
                    app.record(&message);
                }
                return 0;
            }
            TRAY => {
                if l as u32 == WM_LBUTTONDBLCLK {
                    app.show();
                } else if l as u32 == WM_RBUTTONUP {
                    app.tray_menu();
                }
                return 0;
            }
            WM_SIZE if w == SIZE_MINIMIZED as usize => {
                ShowWindow(hwnd, SW_HIDE);
                return 0;
            }
            WM_SIZE => {
                app.update_scroll();
                return 0;
            }
            WM_MOUSEWHEEL if app.active_page == 1 => {
                return SendMessageW(app.settings_pane, message, w, l);
            }
            WM_VSCROLL => {
                let mut info = SCROLLINFO {
                    cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                    fMask: SIF_ALL,
                    ..std::mem::zeroed()
                };
                GetScrollInfo(hwnd, SB_VERT, &mut info);
                let target = match (w & 0xffff) as i32 {
                    SB_LINEUP => app.scroll - app.px(28),
                    SB_LINEDOWN => app.scroll + app.px(28),
                    SB_PAGEUP => app.scroll - info.nPage as i32,
                    SB_PAGEDOWN => app.scroll + info.nPage as i32,
                    SB_THUMBTRACK | SB_THUMBPOSITION => info.nTrackPos,
                    SB_TOP => 0,
                    SB_BOTTOM => info.nMax,
                    _ => app.scroll,
                };
                app.scroll_to(target);
                return 0;
            }
            WM_CLOSE => {
                ShowWindow(hwnd, SW_HIDE);
                return 0;
            }
            WM_POWERBROADCAST => {
                if let Some(wifi) = &app.wifi {
                    if w == PBT_APMSUSPEND as usize {
                        wifi.suspend();
                    } else if w == PBT_APMRESUMEAUTOMATIC as usize
                        || w == PBT_APMRESUMESUSPEND as usize
                    {
                        wifi.resume();
                    }
                }
                if let Some(handle) = &app.handle {
                    if w == PBT_APMSUSPEND as usize {
                        handle.signal.suspend();
                    } else if w == PBT_APMRESUMEAUTOMATIC as usize
                        || w == PBT_APMRESUMESUSPEND as usize
                    {
                        handle.signal.resume();
                    }
                }
                return 1;
            }
            WM_QUERYENDSESSION => {
                if let Some(handle) = &app.handle {
                    handle.signal.stop();
                }
                return 1;
            }
            WM_ENDSESSION if w != 0 => {
                app.handle.take();
                DestroyWindow(hwnd);
                return 0;
            }
            WM_DESTROY => {
                app.wifi.take();
                app.handle.take();
                Shell_NotifyIconW(NIM_DELETE, &app.tray);
                if app.power != 0 {
                    windows_sys::Win32::System::Power::UnregisterSuspendResumeNotification(
                        app.power,
                    );
                }
                DeleteObject(app.font);
                DeleteObject(app.title_font);
                DeleteObject(app.heading_font);
                if !app.header_icon.is_null() {
                    DestroyIcon(app.header_icon);
                }
                PostQuitMessage(0);
                return 0;
            }
            _ => {}
        }
    }
    DefWindowProcW(hwnd, message, w, l)
}
unsafe fn system_dpi() -> i32 {
    let dc = GetDC(null_mut());
    let dpi = GetDeviceCaps(dc, LOGPIXELSX as i32);
    ReleaseDC(null_mut(), dc);
    dpi.max(96)
}
struct InstanceGuard(HANDLE);
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
pub fn run() {
    unsafe {
        let login_launch = std::env::args_os().skip(1).any(|arg| arg == "--autostart");
        let handle = windows_sys::Win32::System::Threading::CreateMutexW(
            null(),
            0,
            wide("Local\\BUAALoginNativeV1").as_ptr(),
        );
        if handle.is_null() {
            MessageBoxW(
                null_mut(),
                wide("无法创建单实例保护，请重新启动程序").as_ptr(),
                wide("北航校园网").as_ptr(),
                MB_OK | MB_ICONERROR,
            );
            return;
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            if !login_launch {
                let previous = FindWindowW(wide("BUAALoginNativeWindow").as_ptr(), null());
                if !previous.is_null() {
                    ShowWindow(previous, SW_RESTORE);
                    SetForegroundWindow(previous);
                }
            }
            CloseHandle(handle);
            return;
        }
        let _instance = InstanceGuard(handle);
        let common = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_STANDARD_CLASSES | ICC_TAB_CLASSES,
        };
        InitCommonControlsEx(&common);
        let instance = GetModuleHandleW(null());
        let class = wide("BUAALoginNativeWindow");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hIcon: LoadIconW(null_mut(), IDI_INFORMATION),
            hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        let pane_name = wide("BUAALoginSettingsPane");
        let pane_class = WNDCLASSW {
            lpfnWndProc: Some(settings_proc),
            hInstance: instance,
            lpszClassName: pane_name.as_ptr(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
            ..std::mem::zeroed()
        };
        RegisterClassW(&pane_class);
        let mut app = Box::new(App::new());
        let dpi = system_dpi();
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_VSCROLL;
        let mut frame = RECT {
            left: 0,
            top: 0,
            right: 680 * dpi / 96,
            bottom: 600 * dpi / 96,
        };
        AdjustWindowRectEx(&mut frame, style, 0, 0);
        let mut work: RECT = std::mem::zeroed();
        SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut work as *mut _ as _, 0);
        let window = CreateWindowExW(
            0,
            class.as_ptr(),
            wide("北航校园网").as_ptr(),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            frame.right - frame.left,
            (frame.bottom - frame.top).min(work.bottom - work.top),
            null_mut(),
            null_mut(),
            instance,
            app.as_mut() as *mut _ as _,
        );
        if window.is_null() {
            return;
        }
        let corner = windows_sys::Win32::Graphics::Dwm::DWMWCP_ROUND;
        windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(
            window,
            33,
            &corner as *const _ as _,
            std::mem::size_of_val(&corner) as u32,
        );
        if login_launch {
            let remembered = SendMessageW(app.remember, BM_GETCHECK, 0, 0) == BST_CHECKED as isize;
            if remembered && app.complete_configuration().is_some() {
                app.start_worker(false);
                if app.handle.is_none() {
                    ShowWindow(window, SW_SHOW);
                    app.show_page(1);
                }
            } else {
                ShowWindow(window, SW_SHOW);
                app.show_page(1);
                app.record("开机自启：请保存完整账号、密码和连接配置后启用自动连接。");
            }
        } else {
            ShowWindow(window, SW_SHOW);
            app.start_worker(true);
        }
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            if IsDialogMessageW(window, &message) == 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
}
