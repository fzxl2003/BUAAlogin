use crate::appearance::{self, Palette};
use buaa_core::{
    engine::{Handle, Mode, RecoveryMode, TestKind},
    Config, DashboardSnapshot, Event, TaskScheduleReason,
};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    net::Ipv4Addr,
    path::PathBuf,
    ptr::{null, null_mut},
    sync::mpsc::{self, Receiver},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{
        LibraryLoader::*,
        SystemInformation::GetLocalTime,
        SystemServices::{SS_ENDELLIPSIS, SS_OWNERDRAW},
        Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime},
    },
    UI::{
        Controls::*,
        Input::KeyboardAndMouse::{EnableWindow, GetFocus, SetFocus},
        Shell::*,
        WindowsAndMessaging::*,
    },
};
const EVENTS: u32 = WM_APP + 1;
const TRAY: u32 = WM_APP + 2;
const WIFI_EVENTS: u32 = WM_APP + 3;
const MODAL_COMMAND: u32 = WM_APP + 20;
const MODAL_CLOSE: u32 = WM_APP + 21;
const PAGES: usize = 32;
const AUTOSTART: usize = 36;
const REPAIR: usize = 37;
const WEB_REPAIR: usize = 38;
const BROWSER_FALLBACK: usize = 39;
const PORTAL_AUTH: usize = 41;
const CONNECTIVITY_PROBE: usize = 42;
const TEST_PORTAL: usize = 43;
const TEST_CONNECTIVITY: usize = 44;
const TEST_BROWSER: usize = 45;
const OPEN_LOGS: usize = 46;
const CONNECTION_SETTINGS: usize = 47;
const ACCOUNT_SETTINGS: usize = 48;
const SAVE_SETTINGS: usize = 49;
const START: usize = 10;
const STOP: usize = 11;
const SCAN: usize = 12;
const HIDE: usize = 13;
const SHOW: usize = 14;
const QUIT: usize = 15;
const REMEMBER: usize = 16;

#[derive(Clone)]
struct InterfaceOption {
    binding: String,
    name: String,
    label: String,
    message: String,
    wifi_id: Option<String>,
}
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
type ControlPosition = (HWND, i32, i32, i32, i32);
struct App {
    window: HWND,
    username: HWND,
    password: HWND,
    interval: HWND,
    gateway: HWND,
    interfaces: HWND,
    remember: HWND,
    start: HWND,
    repair: HWND,
    web_repair: HWND,
    browser_fallback: HWND,
    portal_auth: HWND,
    connectivity_probe: HWND,
    test_buttons: [HWND; 3],
    task_results: [HWND; 3],
    task_next: [HWND; 3],
    browser_path: HWND,
    scan: HWND,
    status: HWND,
    history: HWND,
    traffic_progress: HWND,
    traffic_label: HWND,
    traffic_total: HWND,
    account_value: HWND,
    tabs: HWND,
    page_controls: RefCell<Vec<(HWND, usize)>>,
    building_page: Cell<Option<usize>>,
    settings_positions: RefCell<Vec<ControlPosition>>,
    settings_pane: HWND,
    settings_scroll: i32,
    active_page: usize,
    startup: HWND,
    history_button: HWND,
    dashboard_values: [HWND; 6],
    state_title: HWND,
    state_icon: HWND,
    palette: Palette,
    header_icon: HICON,
    state_kind: Cell<u8>,
    title_font: HFONT,
    heading_font: HFONT,
    dpi: i32,
    scroll: i32,
    selected_bindings: HashSet<String>,
    interface_gateways: HashMap<String, Ipv4Addr>,
    interface_options: Vec<InterfaceOption>,
    traffic_used_bytes: Option<u64>,
    lines: Vec<String>,
    handle: Option<Handle>,
    receiver: Option<Receiver<Event>>,
    closing: bool,
    scanning: bool,
    testing: Option<TestKind>,
    tray: NOTIFYICONDATAW,
    font: HFONT,
    power: windows_sys::Win32::System::Power::HPOWERNOTIFY,
    wifi_options: Vec<crate::wifi::Adapter>,
    wifi_networks: HashMap<String, HashSet<String>>,
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
            repair: null_mut(),
            web_repair: null_mut(),
            browser_fallback: null_mut(),
            portal_auth: null_mut(),
            connectivity_probe: null_mut(),
            test_buttons: [null_mut(); 3],
            task_results: [null_mut(); 3],
            task_next: [null_mut(); 3],
            browser_path: null_mut(),
            scan: null_mut(),
            status: null_mut(),
            history: null_mut(),
            traffic_progress: null_mut(),
            traffic_label: null_mut(),
            traffic_total: null_mut(),
            account_value: null_mut(),
            tabs: null_mut(),
            page_controls: RefCell::new(Vec::new()),
            building_page: Cell::new(None),
            settings_positions: RefCell::new(Vec::new()),
            settings_pane: null_mut(),
            settings_scroll: 0,
            active_page: 0,
            startup: null_mut(),
            history_button: null_mut(),
            dashboard_values: [null_mut(); 6],
            state_title: null_mut(),
            state_icon: null_mut(),
            palette: unsafe { Palette::system() },
            header_icon: null_mut(),
            state_kind: Cell::new(0),
            title_font: null_mut(),
            heading_font: null_mut(),
            dpi: 96,
            scroll: 0,
            selected_bindings: HashSet::new(),
            interface_gateways: HashMap::new(),
            interface_options: vec![],
            traffic_used_bytes: None,
            lines: vec![],
            handle: None,
            receiver: None,
            closing: false,
            scanning: false,
            testing: None,
            tray: unsafe { std::mem::zeroed() },
            font: null_mut(),
            power: 0,
            wifi_options: vec![],
            wifi_networks: HashMap::new(),
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
        let is_push = class == "BUTTON" && (style & BS_TYPEMASK as u32) <= BS_DEFPUSHBUTTON as u32;
        let style = if is_push {
            (style & !(BS_TYPEMASK as u32)) | BS_OWNERDRAW as u32
        } else {
            style
        };
        let style = if class == "BUTTON" {
            style | BS_NOTIFY as u32
        } else {
            style
        };
        let field = class == "EDIT" && self.building_page.get() == Some(1);
        let (x, y, width, height) = if field {
            (x + 10, y + 5, width - 20, height - 10)
        } else {
            (x, y, width, height)
        };
        let style = if field { style & !WS_BORDER } else { style };
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
        }
        SendMessageW(hwnd, WM_SETFONT, self.font as usize, 1);
        SetWindowTheme(hwnd, wide("Explorer").as_ptr(), null());
        hwnd
    }
    fn px(&self, value: i32) -> i32 {
        (value * self.dpi + 48) / 96
    }
    fn rect(&self, x: i32, y: i32, width: i32, height: i32, scroll: i32) -> RECT {
        RECT {
            left: self.px(x),
            top: self.px(y) - scroll,
            right: self.px(x + width),
            bottom: self.px(y + height) - scroll,
        }
    }
    unsafe fn paint(&self, hwnd: HWND, settings: bool) {
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let dc = BeginPaint(hwnd, &mut ps);
        let saved = SaveDC(dc);
        let p = self.palette;
        let mut area: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut area);
        appearance::fill(dc, &area, if settings { p.surface } else { p.background });
        if settings {
            for (control, x, y, width, height) in [
                (self.traffic_total, 170, 90, 180, 30),
                (self.browser_path, 20, 216, 544, 30),
            ] {
                appearance::rounded(
                    dc,
                    &self.rect(x, y, width, height, self.settings_scroll),
                    self.px(8),
                    p.surface,
                    if GetFocus() == control {
                        p.accent
                    } else {
                        p.border
                    },
                );
            }
            let y = self.px(296) - self.settings_scroll;
            appearance::line(dc, self.px(20), y, self.px(564), y, p.border);
        } else if self.active_page == 0 {
            for (y, height) in [(80, 98), (190, 144), (346, 124)] {
                appearance::rounded(
                    dc,
                    &self.rect(32, y, 616, height, self.scroll),
                    self.px(16),
                    p.surface,
                    p.border,
                );
            }
        }
        RestoreDC(dc, saved);
        EndPaint(hwnd, &ps);
    }
    unsafe fn control_color(&self, dc: HDC, control: HWND, edit: bool) -> LRESULT {
        let p = self.palette;
        let prominent = control == self.state_title
            || self.dashboard_values.contains(&control)
            || SendMessageW(control, WM_GETFONT, 0, 0) == self.heading_font as isize;
        SetTextColor(dc, if edit || prominent { p.text } else { p.muted });
        let mut area: RECT = std::mem::zeroed();
        GetWindowRect(control, &mut area);
        MapWindowPoints(
            null_mut(),
            self.window,
            &mut area as *mut RECT as *mut POINT,
            2,
        );
        let y = area.top + self.scroll;
        let card = GetParent(control) == self.settings_pane
            || control == self.history
            || edit
            || (self.active_page == 0 && y >= self.px(80) && y < self.px(470));
        let color = if card { p.surface } else { p.background };
        SetBkColor(dc, color);
        SetBkMode(dc, TRANSPARENT as i32);
        appearance::brush(dc, color) as isize
    }
    unsafe fn draw_item(&self, item: &DRAWITEMSTRUCT) {
        let dc = item.hDC;
        let saved = SaveDC(dc);
        let p = self.palette;
        let mut rect = item.rcItem;
        SetBkMode(dc, TRANSPARENT as i32);
        if item.hwndItem == self.state_icon {
            let color = match self.state_kind.get() {
                1 => p.success,
                2 => p.warning,
                _ => p.accent,
            };
            let bg = if p.high_contrast { p.surface } else { p.soft };
            appearance::fill(dc, &rect, p.surface);
            appearance::rounded(dc, &rect, self.px(18), bg, bg);
            // The symbol has a textual status alongside it, so color is never the only cue.
            let pen = CreatePen(PS_SOLID, self.px(2).max(2), color);
            let old_pen = SelectObject(dc, pen);
            let old_brush = SelectObject(dc, GetStockObject(NULL_BRUSH));
            let x = rect.left;
            let y = rect.top;
            match self.state_kind.get() {
                1 => {
                    MoveToEx(dc, x + self.px(12), y + self.px(24), null_mut());
                    LineTo(dc, x + self.px(21), y + self.px(32));
                    LineTo(dc, x + self.px(36), y + self.px(16));
                }
                2 => {
                    MoveToEx(dc, x + self.px(24), y + self.px(12), null_mut());
                    LineTo(dc, x + self.px(24), y + self.px(28));
                    Ellipse(
                        dc,
                        x + self.px(23),
                        y + self.px(34),
                        x + self.px(26),
                        y + self.px(37),
                    );
                }
                3 => {
                    for xx in [19, 29] {
                        MoveToEx(dc, x + self.px(xx), y + self.px(15), null_mut());
                        LineTo(dc, x + self.px(xx), y + self.px(34));
                    }
                }
                _ => {
                    Arc(
                        dc,
                        x + self.px(8),
                        y + self.px(13),
                        x + self.px(40),
                        y + self.px(42),
                        x + self.px(40),
                        y + self.px(23),
                        x + self.px(8),
                        y + self.px(23),
                    );
                    Arc(
                        dc,
                        x + self.px(15),
                        y + self.px(22),
                        x + self.px(33),
                        y + self.px(39),
                        x + self.px(33),
                        y + self.px(28),
                        x + self.px(15),
                        y + self.px(28),
                    );
                    Ellipse(
                        dc,
                        x + self.px(23),
                        y + self.px(34),
                        x + self.px(26),
                        y + self.px(37),
                    );
                }
            }
            SelectObject(dc, old_brush);
            SelectObject(dc, old_pen);
            DeleteObject(pen);
        } else {
            let tab = item.hwndItem == self.tabs;
            let selected = if tab {
                item.itemID as usize == self.active_page
            } else {
                item.itemState & ODS_SELECTED != 0
            };
            let disabled = item.itemState & ODS_DISABLED != 0;
            let primary = item.CtlID as usize == START;
            let background = if GetParent(item.hwndItem) == self.settings_pane {
                p.surface
            } else {
                p.background
            };
            appearance::fill(dc, &rect, background);
            let contrast_selected = p.high_contrast && tab && selected;
            let fill = if disabled {
                p.background
            } else if primary || contrast_selected {
                p.accent
            } else if selected {
                p.soft
            } else {
                p.surface
            };
            let border = if primary && !disabled {
                p.accent
            } else if tab && !selected {
                background
            } else {
                p.border
            };
            if tab {
                InflateRect(&mut rect, -self.px(2), -self.px(2));
            }
            appearance::rounded(dc, &rect, self.px(12), fill, border);
            if selected && !tab {
                OffsetRect(&mut rect, 0, self.px(1));
            }
            SetTextColor(
                dc,
                if disabled {
                    GetSysColor(COLOR_GRAYTEXT)
                } else if primary || contrast_selected {
                    p.accent_text
                } else if tab && selected {
                    p.accent
                } else {
                    p.text
                },
            );
            SelectObject(
                dc,
                if primary || (tab && selected) {
                    self.heading_font
                } else {
                    self.font
                },
            );
            let mut chars = [0u16; 256];
            let len = if tab {
                let label = if item.itemID == 0 { "连接" } else { "设置" };
                let mut count = 0;
                for ch in label.encode_utf16() {
                    chars[count] = ch;
                    count += 1;
                }
                count as i32
            } else {
                GetWindowTextW(item.hwndItem, chars.as_mut_ptr(), chars.len() as i32)
            };
            DrawTextW(
                dc,
                chars.as_ptr(),
                len,
                &mut rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
            if item.itemState & ODS_FOCUS != 0 && item.itemState & ODS_NOFOCUSRECT == 0 {
                InflateRect(&mut rect, -self.px(4), -self.px(4));
                DrawFocusRect(dc, &rect);
            }
        }
        RestoreDC(dc, saved);
    }
    unsafe fn paint_navigation(&self, hwnd: HWND) {
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let dc = BeginPaint(hwnd, &mut ps);
        let saved = SaveDC(dc);
        let mut area: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut area);
        let p = self.palette;
        appearance::fill(dc, &area, p.background);
        appearance::rounded(dc, &area, self.px(12), p.soft, p.soft);
        SetBkMode(dc, TRANSPARENT as i32);
        for (index, title) in ["连接", "设置"].iter().enumerate() {
            let mut rect: RECT = std::mem::zeroed();
            SendMessageW(hwnd, TCM_GETITEMRECT, index, &mut rect as *mut _ as isize);
            let selected = index == self.active_page;
            if selected {
                let fill = if p.high_contrast { p.accent } else { p.surface };
                appearance::rounded(dc, &rect, self.px(10), fill, p.border);
            }
            SelectObject(
                dc,
                if selected {
                    self.heading_font
                } else {
                    self.font
                },
            );
            SetTextColor(
                dc,
                if selected && p.high_contrast {
                    p.accent_text
                } else if selected {
                    p.accent
                } else {
                    p.muted
                },
            );
            let title = wide(title);
            DrawTextW(
                dc,
                title.as_ptr(),
                -1,
                &mut rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );
            if selected
                && GetFocus() == hwnd
                && SendMessageW(hwnd, WM_QUERYUISTATE, 0, 0) & UISF_HIDEFOCUS as isize == 0
            {
                InflateRect(&mut rect, -self.px(4), -self.px(4));
                DrawFocusRect(dc, &rect);
            }
        }
        RestoreDC(dc, saved);
        EndPaint(hwnd, &ps);
    }
    unsafe fn label(&self, text: &str, x: i32, y: i32, width: i32) -> HWND {
        self.control("STATIC", text, 0, x, y, width, 22, 0)
    }
    unsafe fn build(&mut self) {
        cleanup_logs();
        self.dpi = system_dpi();
        // Explicit UI family avoids the classic bitmap/serif fallback on Chinese Windows.
        // Three shared fonts, the same count as the previous UI.
        for (slot, height, weight) in [
            (&mut self.font, 14, FW_NORMAL),
            (&mut self.heading_font, 15, FW_SEMIBOLD),
            (&mut self.title_font, 24, FW_SEMIBOLD),
        ] {
            *slot = CreateFontW(
                -(height * self.dpi / 96),
                0,
                0,
                0,
                weight as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                CLEARTYPE_QUALITY as u32,
                DEFAULT_PITCH as u32,
                wide("Microsoft YaHei UI").as_ptr(),
            );
        }
        // Keep native tabs for keyboard/accessibility, draw only their small segmented header.
        self.tabs = self.control(
            "SysTabControl32",
            "",
            WS_TABSTOP | WS_CLIPSIBLINGS | TCS_FIXEDWIDTH,
            200,
            22,
            280,
            38,
            PAGES,
        );
        SetWindowTheme(self.tabs, wide("").as_ptr(), wide("").as_ptr());
        SendMessageW(
            self.tabs,
            TCM_SETITEMSIZE,
            0,
            ((self.px(32) as isize) << 16) | self.px(134) as isize,
        );
        SendMessageW(self.tabs, TCM_SETPADDING, 0, 0);
        for (index, title) in ["连接", "设置"].iter().enumerate() {
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
        SetWindowSubclass(
            self.tabs,
            Some(navigation_proc),
            1,
            self as *const _ as usize,
        );
        self.settings_pane = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            wide("BUAALoginSettingsPane").as_ptr(),
            wide("").as_ptr(),
            WS_CHILD | WS_VSCROLL | WS_CLIPCHILDREN,
            self.px(32),
            self.px(80),
            self.px(616),
            self.px(524),
            self.window,
            null_mut(),
            GetModuleHandleW(null()),
            self as *mut _ as _,
        );
        self.building_page.set(Some(0));
        self.state_icon = self.control("STATIC", "连接状态", SS_OWNERDRAW, 52, 98, 48, 48, 0);
        self.state_title =
            self.control("STATIC", "北航校园网", SS_ENDELLIPSIS, 120, 94, 504, 36, 0);
        SendMessageW(self.state_title, WM_SETFONT, self.title_font as usize, 1);
        self.status = self.control(
            "STATIC",
            "请设置账号并检测校园网接口。",
            0,
            120,
            135,
            504,
            36,
            0,
        );
        self.update_state("idle");
        for (index, title) in [
            "账号",
            "当前接口",
            "检查周期",
            "上次检查",
            "本地 IP",
            "校园网 IP",
        ]
        .iter()
        .enumerate()
        {
            let x = 48 + (index as i32 % 3) * 202;
            let y = 200 + (index as i32 / 3) * 50;
            self.label(title, x, y, 180);
            self.dashboard_values[index] =
                self.control("STATIC", "—", SS_ENDELLIPSIS, x, y + 20, 180, 24, 0);
            SendMessageW(
                self.dashboard_values[index],
                WM_SETFONT,
                self.heading_font as usize,
                1,
            );
        }
        self.account_value = self.dashboard_values[0];
        for control in [self.dashboard_values[0], self.dashboard_values[1]] {
            SetWindowPos(
                control,
                null_mut(),
                0,
                0,
                self.px(84),
                self.px(24),
                SWP_NOMOVE | SWP_NOZORDER,
            );
        }
        self.control(
            "BUTTON",
            "设置…",
            WS_TABSTOP,
            144,
            218,
            82,
            29,
            ACCOUNT_SETTINGS,
        );
        self.control(
            "BUTTON",
            "接口设置…",
            WS_TABSTOP,
            342,
            218,
            94,
            29,
            CONNECTION_SETTINGS,
        );

        self.label("流量使用情况", 48, 300, 160);
        self.traffic_progress =
            self.control("msctls_progress32", "", PBS_SMOOTH, 48, 320, 584, 10, 0);
        SendMessageW(self.traffic_progress, PBM_SETRANGE32, 0, 1000);
        self.traffic_label = self.control(
            "STATIC",
            "尚未获取流量",
            SS_ENDELLIPSIS,
            208,
            300,
            424,
            24,
            0,
        );
        self.portal_auth = self.control(
            "BUTTON",
            "校园网认证（普通连接）",
            BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
            48,
            362,
            190,
            25,
            PORTAL_AUTH,
        );
        self.connectivity_probe = self.control(
            "BUTTON",
            "外网可用性检测",
            BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
            48,
            398,
            190,
            25,
            CONNECTIVITY_PROBE,
        );
        self.browser_fallback = self.control(
            "BUTTON",
            "网页兜底",
            BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
            48,
            434,
            190,
            25,
            BROWSER_FALLBACK,
        );
        for control in [
            self.portal_auth,
            self.connectivity_probe,
            self.browser_fallback,
        ] {
            SendMessageW(control, BM_SETCHECK, BST_CHECKED as usize, 0);
        }
        for (index, y) in [362, 398, 434].iter().enumerate() {
            self.task_results[index] =
                self.control("STATIC", "尚未运行", SS_ENDELLIPSIS, 250, *y, 156, 24, 0);
            self.task_next[index] = self.control(
                "STATIC",
                if index == 2 { "—" } else { "启动后立即" },
                SS_ENDELLIPSIS,
                414,
                *y,
                130,
                24,
                0,
            );
            self.test_buttons[index] = self.control(
                "BUTTON",
                "测试",
                WS_TABSTOP,
                548,
                *y - 3,
                68,
                29,
                TEST_PORTAL + index,
            );
        }
        self.start = self.control(
            "BUTTON",
            "设置账号",
            BS_DEFPUSHBUTTON as u32 | WS_TABSTOP,
            462,
            486,
            186,
            38,
            START,
        );

        // These hidden native controls keep sensitive/configuration values out of the
        // main pages. They are edited through the account and interface dialogs.
        self.building_page.set(Some(2));
        self.username = self.control("EDIT", "", ES_AUTOHSCROLL as u32, 0, 0, 1, 1, 20);
        self.password = self.control("EDIT", "", ES_PASSWORD as u32, 0, 0, 1, 1, 21);
        self.remember = self.control("BUTTON", "", BS_AUTOCHECKBOX as u32, 0, 0, 1, 1, REMEMBER);
        SendMessageW(self.remember, BM_SETCHECK, BST_CHECKED as usize, 0);
        self.interval = self.control("EDIT", "300", ES_NUMBER as u32, 0, 0, 1, 1, 22);
        self.gateway = self.control("EDIT", "10.200.21.4", ES_AUTOHSCROLL as u32, 0, 0, 1, 1, 24);
        self.interfaces = null_mut();

        self.building_page.set(Some(1));
        let title = self.label("通用设置", 20, 14, 560);
        SendMessageW(title, WM_SETFONT, self.heading_font as usize, 1);
        self.startup = self.control(
            "BUTTON",
            "开机自启",
            BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
            20,
            48,
            180,
            24,
            AUTOSTART,
        );
        self.control(
            "STATIC",
            "登录系统后自动启动；需先在账号弹窗中保存完整凭据。",
            0,
            210,
            49,
            354,
            22,
            0,
        );
        self.label("总流量（GB）", 20, 96, 140);
        self.traffic_total = self.control(
            "EDIT",
            "",
            WS_BORDER | ES_AUTOHSCROLL as u32 | WS_TABSTOP,
            170,
            90,
            180,
            30,
            50,
        );
        self.label("留空时只显示已用流量，不计算百分比。", 368, 96, 196);
        self.label("检查周期（秒）", 20, 146, 140);
        let interval_setting = self.control(
            "EDIT",
            "300",
            WS_BORDER | ES_NUMBER as u32 | WS_TABSTOP,
            170,
            140,
            180,
            30,
            51,
        );
        SetWindowTextW(interval_setting, wide(&text(self.interval)).as_ptr());
        self.label("10–86400", 368, 146, 196);
        self.label(
            "浏览器可执行文件（留空自动查找 Chrome / Edge / Chromium）",
            20,
            188,
            544,
        );
        self.browser_path = self.control(
            "EDIT",
            "",
            WS_BORDER | ES_AUTOHSCROLL as u32 | WS_TABSTOP,
            20,
            216,
            544,
            30,
            40,
        );
        self.control(
            "BUTTON",
            "保存设置",
            WS_TABSTOP,
            444,
            260,
            120,
            30,
            SAVE_SETTINGS,
        );
        let recent = self.label("最近活动", 20, 314, 300);
        SendMessageW(recent, WM_SETFONT, self.heading_font as usize, 1);
        self.history_button = self.control(
            "BUTTON",
            "打开日志文件夹",
            WS_TABSTOP,
            408,
            306,
            156,
            30,
            OPEN_LOGS,
        );
        self.history = self.control(
            "EDIT",
            "暂无活动记录",
            ES_MULTILINE as u32
                | ES_READONLY as u32
                | ES_AUTOVSCROLL as u32
                | WS_VSCROLL
                | WS_TABSTOP,
            20,
            352,
            544,
            240,
            0,
        );
        self.scan = null_mut();
        if let Ok(path) = wifi_path() {
            if let Ok(data) = std::fs::read(path) {
                self.wifi_networks = serde_json::from_slice(&data).unwrap_or_else(|_| {
                    serde_json::from_slice::<HashSet<String>>(&data)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|id| {
                            (
                                id,
                                HashSet::from([
                                    "BUAA-WiFi".to_owned(),
                                    "BUAA-WiFi-Free".to_owned(),
                                ]),
                            )
                        })
                        .collect()
                });
            }
        }
        self.wifi = Some(crate::wifi::Wifi::new(self.window, WIFI_EVENTS));
        self.refresh_wifi_options();
        self.building_page.set(None);
        self.update_settings_scroll();
        self.show_page(0);
        self.tray.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        self.tray.hWnd = self.window;
        self.tray.uID = 1;
        self.tray.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        self.tray.uCallbackMessage = TRAY;
        self.header_icon = appearance::app_icon();
        self.tray.hIcon = if self.header_icon.is_null() {
            LoadIconW(null_mut(), IDI_APPLICATION)
        } else {
            self.header_icon
        };
        SendMessageW(
            self.window,
            WM_SETICON,
            ICON_BIG as usize,
            self.tray.hIcon as isize,
        );
        SendMessageW(
            self.window,
            WM_SETICON,
            ICON_SMALL as usize,
            self.tray.hIcon as isize,
        );
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
        let (title, kind) = match state {
            "online" => ("网络正常", 1),
            "suspect" => ("外网异常，正在复核", 2),
            "protocol_recovery" => ("协议恢复中", 0),
            "browser_recovery" => ("后台网页兜底中", 0),
            "verifying" => ("正在验证外网", 0),
            "cooldown" => ("恢复冷却中", 2),
            "needs_user" => ("认证需要处理", 2),
            "retry" => ("等待自动重试", 2),
            "error" | "failed" => ("连接需要处理", 2),
            "suspended" => ("睡眠期间暂停", 3),
            "waiting_network" => ("等待网络", 0),
            "checking" => ("正在检查连接", 0),
            "interfaces" => ("接口检测完成", 1),
            "stopped" => ("已停止自动重连", 3),
            _ => ("北航校园网", 0),
        };
        self.state_kind.set(kind);
        let can_repair = self.handle.is_some()
            && !self.scanning
            && !matches!(
                state,
                "protocol_recovery"
                    | "browser_recovery"
                    | "verifying"
                    | "cooldown"
                    | "needs_user"
                    | "suspended"
                    | "waiting_network"
                    | "stopped"
            );
        let _ = can_repair;
        SetWindowTextW(self.state_title, wide(title).as_ptr());
        InvalidateRect(self.state_icon, null(), 0);
    }
    unsafe fn show_page(&mut self, index: usize) {
        self.active_page = index;
        SendMessageW(self.tabs, TCM_SETCURSEL, index, 0);
        for &(control, page) in self.page_controls.borrow().iter() {
            ShowWindow(control, if page == index { SW_SHOW } else { SW_HIDE });
        }
        ShowWindow(
            self.settings_pane,
            if index == 1 { SW_SHOW } else { SW_HIDE },
        );
        if index == 1 {
            self.render_history();
        }
        self.resize_for_page(index);
        SetFocus(self.tabs);
        InvalidateRect(self.tabs, null(), 1);
        InvalidateRect(self.window, null(), 1);
    }
    unsafe fn resize_for_page(&self, index: usize) {
        let style =
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_VSCROLL | WS_CLIPCHILDREN;
        let mut frame = RECT {
            left: 0,
            top: 0,
            right: self.px(680),
            bottom: self.px(if index == 0 { 540 } else { 660 }),
        };
        AdjustWindowRectEx(&mut frame, style, 0, 0);
        SetWindowPos(
            self.window,
            null_mut(),
            0,
            0,
            frame.right - frame.left,
            frame.bottom - frame.top,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
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
            nMax: self.px(612) - 1,
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
        InvalidateRect(self.settings_pane, null(), 0);
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
        if field == self.username || field == self.password {
            self.show_account_dialog();
            return;
        }
        if field == self.gateway {
            self.show_interface_dialog();
            return;
        }
        self.show_page(1);
        if field == self.interval {
            let visible = GetDlgItem(self.settings_pane, 51);
            SetFocus(visible);
            SendMessageW(visible, EM_SETSEL, 0, -1);
            return;
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
        self.configuration(true)
    }
    unsafe fn configuration(&self, require_credentials: bool) -> Option<Config> {
        let config = Config {
            username: text(self.username).trim().into(),
            password: text(self.password).into(),
            interval: text(self.interval).parse().ok()?,
            gateway_ip: text(self.gateway).trim().parse().ok()?,
            portal_auth_enabled: SendMessageW(self.portal_auth, BM_GETCHECK, 0, 0)
                == BST_CHECKED as isize,
            connectivity_probe_enabled: SendMessageW(self.connectivity_probe, BM_GETCHECK, 0, 0)
                == BST_CHECKED as isize,
            browser_fallback: SendMessageW(self.browser_fallback, BM_GETCHECK, 0, 0)
                == BST_CHECKED as isize,
            browser_path: Some(text(self.browser_path).trim().to_owned())
                .filter(|path| !path.is_empty()),
            interfaces: self.selected_bindings.iter().cloned().collect(),
            interface_gateways: self.interface_gateways.clone(),
            ..Config::default()
        };
        config.validate(require_credentials).ok()?;
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
    unsafe fn update_start_title(&self) {
        if self.start.is_null() {
            return;
        }
        let credentials_required = SendMessageW(self.portal_auth, BM_GETCHECK, 0, 0)
            == BST_CHECKED as isize
            || SendMessageW(self.browser_fallback, BM_GETCHECK, 0, 0) == BST_CHECKED as isize;
        let configured = !credentials_required
            || (!text(self.username).trim().is_empty() && !text(self.password).is_empty());
        SetWindowTextW(
            self.start,
            wide(if self.handle.is_some() && !self.scanning {
                "停止自动连接"
            } else if configured {
                "开始自动连接"
            } else {
                "设置账号"
            })
            .as_ptr(),
        );
        let username = text(self.username);
        let account = if username.trim().is_empty() {
            "未设置"
        } else {
            username.trim()
        };
        SetWindowTextW(self.account_value, wide(account).as_ptr());
        if !self.dashboard_values[2].is_null() && self.handle.is_none() {
            SetWindowTextW(
                self.dashboard_values[2],
                wide(&format!("{} 秒", text(self.interval))).as_ptr(),
            );
        }
    }
    unsafe fn dashboard_action(&mut self) {
        if self.handle.is_some() && !self.scanning {
            self.stop_worker();
            return;
        }
        let credentials_required = SendMessageW(self.portal_auth, BM_GETCHECK, 0, 0)
            == BST_CHECKED as isize
            || SendMessageW(self.browser_fallback, BM_GETCHECK, 0, 0) == BST_CHECKED as isize;
        if credentials_required
            && (text(self.username).trim().is_empty() || text(self.password).is_empty())
        {
            self.focus_setting(self.username);
        } else {
            self.start_worker(false);
        }
    }
    unsafe fn apply_dashboard(&mut self, snapshot: &DashboardSnapshot, message: &str) {
        self.update_state(&snapshot.phase);
        let values = [
            if text(self.username).trim().is_empty() {
                "未设置".into()
            } else {
                text(self.username).trim().into()
            },
            snapshot.interface.clone().unwrap_or_else(|| "—".into()),
            format!("{} 秒", snapshot.interval),
            local_time(snapshot.last_check_at),
            snapshot
                .local_ip
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "—".into()),
            snapshot.campus_ip.clone().unwrap_or_else(|| "—".into()),
        ];
        for (control, value) in self.dashboard_values.iter().zip(values) {
            SetWindowTextW(*control, wide(&value).as_ptr());
        }
        if let Some(used) = snapshot.traffic_used_bytes {
            self.traffic_used_bytes = Some(used);
        }
        self.update_traffic();
        if snapshot.phase != "stopped" {
            for (index, task) in [
                &snapshot.tasks.portal_auth,
                &snapshot.tasks.connectivity,
                &snapshot.tasks.browser_fallback,
            ]
            .iter()
            .enumerate()
            {
                let result = match index {
                    0 => match snapshot.authentication {
                        buaa_core::connectivity::Authentication::Online => "🟢 已认证".into(),
                        buaa_core::connectivity::Authentication::Offline => "🟠 未认证".into(),
                        buaa_core::connectivity::Authentication::Unknown => "⚪ 未知".into(),
                    },
                    1 => match snapshot.connectivity {
                        buaa_core::connectivity::Connectivity::Available => "🟢 可用".into(),
                        buaa_core::connectivity::Connectivity::Unavailable => "🔴 不可用".into(),
                        buaa_core::connectivity::Connectivity::Unknown => "⚪ 未知".into(),
                    },
                    _ if task.schedule_reason == TaskScheduleReason::WaitingTrigger => "—".into(),
                    _ => task.message.clone(),
                };
                SetWindowTextW(self.task_results[index], wide(&result).as_ptr());
                let next = task
                    .next_run_at
                    .map(|v| format!("下次 {}", local_time(Some(v))))
                    .unwrap_or_else(|| match task.schedule_reason {
                        TaskScheduleReason::Disabled => "已关闭".into(),
                        TaskScheduleReason::OnStart => "启动后立即".into(),
                        TaskScheduleReason::WaitingTrigger => "—".into(),
                        TaskScheduleReason::Cooldown => "冷却中".into(),
                        TaskScheduleReason::Scheduled => "—".into(),
                    });
                SetWindowTextW(self.task_next[index], wide(&next).as_ptr());
            }
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
            nMax: self.px(if self.active_page == 0 { 540 } else { 660 }) - 1,
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
        let start = self.lines.len().saturating_sub(40);
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
        let line = format!("{}  {}", local_time(Some(unix_now())), text);
        self.lines.push(line.clone());
        let _ = append_log(&line);
        if self.lines.len() > 120 {
            self.lines.remove(0);
        }
        if self.active_page == 1 {
            self.render_history();
        }
    }
    unsafe fn connectivity_changed(&self) {
        let enabled =
            SendMessageW(self.connectivity_probe, BM_GETCHECK, 0, 0) == BST_CHECKED as isize;
        if !enabled {
            SendMessageW(
                self.browser_fallback,
                BM_SETCHECK,
                BST_UNCHECKED as usize,
                0,
            );
        }
        EnableWindow(
            self.browser_fallback,
            (enabled && self.handle.is_none()) as i32,
        );
        SetWindowTextW(
            self.task_next[1],
            wide(if enabled {
                "启动后立即"
            } else {
                "已关闭"
            })
            .as_ptr(),
        );
        if !enabled {
            SetWindowTextW(self.task_next[2], wide("已关闭").as_ptr());
        } else {
            SetWindowTextW(
                self.task_next[2],
                wide(
                    if SendMessageW(self.browser_fallback, BM_GETCHECK, 0, 0)
                        == BST_CHECKED as isize
                    {
                        "—"
                    } else {
                        "已关闭"
                    },
                )
                .as_ptr(),
            );
        }
        self.update_start_title();
    }
    unsafe fn run_test(&mut self, kind: TestKind) {
        if kind != TestKind::Connectivity
            && (text(self.username).trim().is_empty() || text(self.password).is_empty())
        {
            self.focus_setting(self.username);
            return;
        }
        self.testing = Some(kind);
        for button in self.test_buttons {
            EnableWindow(button, 0);
        }
        let index = match kind {
            TestKind::PortalAuth => 0,
            TestKind::Connectivity => 1,
            TestKind::Browser => 2,
        };
        SetWindowTextW(self.task_results[index], wide("正在测试…").as_ptr());
        if let Some(handle) = self.handle.as_ref().filter(|_| !self.scanning) {
            handle.signal.test(kind);
            return;
        }
        let Some(config) = self.complete_configuration() else {
            self.testing = None;
            for button in self.test_buttons {
                EnableWindow(button, 1);
            }
            return;
        };
        let (tx, rx) = mpsc::channel();
        let window = self.window as usize;
        match Handle::spawn(config, Mode::Test(kind), move |event| {
            if tx.send(event).is_ok() {
                PostMessageW(window as HWND, EVENTS, 0, 0);
            }
        }) {
            Ok(handle) => {
                self.handle = Some(handle);
                self.receiver = Some(rx);
                self.busy(true);
            }
            Err(error) => {
                self.testing = None;
                for button in self.test_buttons {
                    EnableWindow(button, 1);
                }
                self.record(&error);
            }
        }
    }
    unsafe fn open_log_folder(&mut self) {
        match log_folder().and_then(|path| {
            std::fs::create_dir_all(&path).map_err(|_| "无法创建日志目录".to_string())?;
            Ok(path)
        }) {
            Ok(path) => {
                ShellExecuteW(
                    self.window,
                    wide("open").as_ptr(),
                    wide(path.to_string_lossy().as_ref()).as_ptr(),
                    null(),
                    null(),
                    SW_SHOWNORMAL,
                );
            }
            Err(error) => self.record(&error),
        }
    }
    unsafe fn busy(&self, busy: bool) {
        for c in [
            self.username,
            self.password,
            self.interval,
            self.gateway,
            self.portal_auth,
            self.connectivity_probe,
            self.browser_fallback,
            self.browser_path,
            self.remember,
        ] {
            EnableWindow(c, (!busy) as i32);
        }
        EnableWindow(
            self.start,
            (!self.scanning && self.testing.is_none()) as i32,
        );
        for button in self.test_buttons {
            EnableWindow(button, self.testing.is_none() as i32);
        }
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
            portal_auth_enabled: SendMessageW(self.portal_auth, BM_GETCHECK, 0, 0)
                == BST_CHECKED as isize,
            connectivity_probe_enabled: SendMessageW(self.connectivity_probe, BM_GETCHECK, 0, 0)
                == BST_CHECKED as isize,
            browser_fallback: SendMessageW(self.browser_fallback, BM_GETCHECK, 0, 0)
                == BST_CHECKED as isize,
            browser_path: Some(text(self.browser_path).trim().to_owned())
                .filter(|path| !path.is_empty()),
            interfaces: if scan {
                vec![]
            } else {
                self.selected_bindings.iter().cloned().collect()
            },
            interface_gateways: if scan {
                HashMap::new()
            } else {
                self.interface_gateways.clone()
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
                    wifi.configure(self.wifi_networks.clone(), !scan);
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
                self.refresh_wifi_options();
                self.interface_options = event
                    .records
                    .unwrap_or_default()
                    .into_iter()
                    .map(|row| {
                        let wifi = self.wifi_options.iter().find(|wifi| {
                            wifi.binding.eq_ignore_ascii_case(&row.interface.name)
                                || wifi.name.eq_ignore_ascii_case(&row.interface.name)
                        });
                        InterfaceOption {
                            binding: row.binding,
                            name: row.interface.name,
                            label: row.label,
                            message: row.message,
                            wifi_id: wifi.map(|wifi| wifi.id.clone()),
                        }
                    })
                    .collect();
                let existing: HashSet<_> = self
                    .interface_options
                    .iter()
                    .map(|row| row.name.to_lowercase())
                    .collect();
                for wifi in &self.wifi_options {
                    if !existing.contains(&wifi.binding.to_lowercase())
                        && !existing.contains(&wifi.name.to_lowercase())
                    {
                        self.interface_options.push(InterfaceOption {
                            binding: wifi.binding.clone(),
                            name: wifi.binding.clone(),
                            label: wifi.binding.clone(),
                            message: "未获取 IP，可配置自动连接".into(),
                            wifi_id: Some(wifi.id.clone()),
                        });
                    }
                }
                self.interface_options
                    .sort_by_key(|row| (row.wifi_id.is_none(), row.name.clone()));
            }
            if event.state == "stopped" {
                let was_scanning = self.scanning;
                self.scanning = false;
                if let Some(wifi) = &self.wifi {
                    wifi.configure(self.wifi_networks.clone(), false);
                }
                self.handle.take();
                self.receiver.take();
                self.testing = None;
                self.busy(false);
                self.load_credentials();
                SetWindowTextW(
                    self.task_next[0],
                    wide(
                        if SendMessageW(self.portal_auth, BM_GETCHECK, 0, 0) == BST_CHECKED as isize
                        {
                            "启动后立即"
                        } else {
                            "已关闭"
                        },
                    )
                    .as_ptr(),
                );
                self.connectivity_changed();
                if self.closing {
                    DestroyWindow(self.window);
                    return;
                }
                if was_scanning {
                    continue;
                }
            }
            if event.state == "test_result" {
                if let Some(kind) = self.testing.take() {
                    let index = match kind {
                        TestKind::PortalAuth => 0,
                        TestKind::Connectivity => 1,
                        TestKind::Browser => 2,
                    };
                    SetWindowTextW(self.task_results[index], wide(&event.message).as_ptr());
                    for button in self.test_buttons {
                        EnableWindow(button, 1);
                    }
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
            wifi.configure(self.wifi_networks.clone(), false);
        }
        if let Some(h) = &self.handle {
            h.signal.stop();
            EnableWindow(self.start, 0);
            SetWindowTextW(self.start, wide("正在停止…").as_ptr());
            self.record("正在停止…");
        }
    }
    unsafe fn request_recovery(&self, mode: RecoveryMode) {
        if let Some(handle) = self.handle.as_ref().filter(|_| !self.scanning) {
            handle.signal.recover(mode);
            EnableWindow(self.repair, 0);
            EnableWindow(self.web_repair, 0);
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
        self.wifi_options = crate::wifi::adapters().unwrap_or_default();
        let existing: HashSet<_> = self
            .interface_options
            .iter()
            .map(|row| row.name.to_lowercase())
            .collect();
        for wifi in &self.wifi_options {
            if !existing.contains(&wifi.binding.to_lowercase())
                && !existing.contains(&wifi.name.to_lowercase())
            {
                self.interface_options.push(InterfaceOption {
                    binding: wifi.binding.clone(),
                    name: wifi.binding.clone(),
                    label: wifi.binding.clone(),
                    message: "未获取 IP，可配置自动连接".into(),
                    wifi_id: Some(wifi.id.clone()),
                });
            }
        }
        self.interface_options
            .sort_by_key(|row| (row.wifi_id.is_none(), row.name.clone()));
    }
    unsafe fn modal_window(&self, owner: HWND, title: &str, width: i32, height: i32) -> HWND {
        let mut owner_rect: RECT = std::mem::zeroed();
        GetWindowRect(owner, &mut owner_rect);
        let width = self.px(width);
        let height = self.px(height);
        let dialog = CreateWindowExW(
            WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
            wide("BUAALoginModalWindow").as_ptr(),
            wide(title).as_ptr(),
            WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN,
            owner_rect.left + ((owner_rect.right - owner_rect.left - width) / 2).max(0),
            owner_rect.top + ((owner_rect.bottom - owner_rect.top - height) / 2).max(0),
            width,
            height,
            owner,
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        );
        if !dialog.is_null() {
            SetWindowSubclass(dialog, Some(modal_proc), 1, 0);
        }
        dialog
    }
    #[allow(clippy::too_many_arguments)]
    unsafe fn modal_control(
        &self,
        parent: HWND,
        class: &str,
        text_value: &str,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        id: usize,
    ) -> HWND {
        let control = CreateWindowExW(
            if class == "EDIT" { WS_EX_CLIENTEDGE } else { 0 },
            wide(class).as_ptr(),
            wide(text_value).as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            self.px(x),
            self.px(y),
            self.px(width),
            self.px(height),
            parent,
            id as HMENU,
            GetModuleHandleW(null()),
            null(),
        );
        SendMessageW(control, WM_SETFONT, self.font as usize, 1);
        control
    }
    unsafe fn show_account_dialog(&mut self) {
        if self.handle.is_some() {
            MessageBoxW(
                self.window,
                wide("请先停止自动连接再修改账号。").as_ptr(),
                wide("账号设置").as_ptr(),
                MB_OK | MB_ICONINFORMATION,
            );
            return;
        }
        let dialog = self.modal_window(self.window, "账号设置", 460, 250);
        if dialog.is_null() {
            return;
        }
        self.modal_control(dialog, "STATIC", "校园网账号", 0, 22, 18, 416, 22, 0);
        let username = self.modal_control(
            dialog,
            "EDIT",
            &text(self.username),
            ES_AUTOHSCROLL as u32 | WS_TABSTOP,
            22,
            42,
            416,
            28,
            9001,
        );
        self.modal_control(dialog, "STATIC", "校园网密码", 0, 22, 78, 416, 22, 0);
        let password = self.modal_control(
            dialog,
            "EDIT",
            &text(self.password),
            ES_PASSWORD as u32 | ES_AUTOHSCROLL as u32 | WS_TABSTOP,
            22,
            102,
            416,
            28,
            9002,
        );
        let remember = self.modal_control(
            dialog,
            "BUTTON",
            "记住账号密码（应用内轻量加密）",
            BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
            22,
            142,
            330,
            24,
            9003,
        );
        SendMessageW(
            remember,
            BM_SETCHECK,
            SendMessageW(self.remember, BM_GETCHECK, 0, 0) as usize,
            0,
        );
        self.modal_control(dialog, "BUTTON", "取消", WS_TABSTOP, 246, 176, 88, 30, 2);
        self.modal_control(
            dialog,
            "BUTTON",
            "保存",
            BS_DEFPUSHBUTTON as u32 | WS_TABSTOP,
            346,
            176,
            92,
            30,
            1,
        );
        EnableWindow(self.window, 0);
        ShowWindow(dialog, SW_SHOW);
        SetForegroundWindow(dialog);
        SetFocus(username);
        let mut saved = false;
        let mut message: MSG = std::mem::zeroed();
        'messages: while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            if message.hwnd == dialog && message.message == MODAL_CLOSE {
                break;
            }
            if message.hwnd == dialog && message.message == MODAL_COMMAND {
                match message.wParam & 0xffff {
                    2 => break 'messages,
                    1 => {
                        let name = text(username).trim().to_owned();
                        let secret = text(password);
                        if name.is_empty() || secret.is_empty() {
                            MessageBoxW(
                                dialog,
                                wide("请输入账号和密码").as_ptr(),
                                wide("账号设置").as_ptr(),
                                MB_OK | MB_ICONINFORMATION,
                            );
                            continue;
                        }
                        SetWindowTextW(self.username, wide(&name).as_ptr());
                        SetWindowTextW(self.password, wide(&secret).as_ptr());
                        SendMessageW(
                            self.remember,
                            BM_SETCHECK,
                            SendMessageW(remember, BM_GETCHECK, 0, 0) as usize,
                            0,
                        );
                        if let Some(config) = self.complete_configuration() {
                            if let Err(error) = self.save_credentials(&config) {
                                MessageBoxW(
                                    dialog,
                                    wide(&error).as_ptr(),
                                    wide("账号设置").as_ptr(),
                                    MB_OK | MB_ICONERROR,
                                );
                                continue;
                            }
                        }
                        saved = true;
                        break 'messages;
                    }
                    _ => {}
                }
            }
            if IsDialogMessageW(dialog, &message) == 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        DestroyWindow(dialog);
        EnableWindow(self.window, 1);
        SetForegroundWindow(self.window);
        self.update_start_title();
        if saved {
            self.record("账号设置已保存");
        }
    }
    unsafe fn show_interface_detail_dialog(&mut self, owner: HWND, index: usize) {
        let Some(option) = self.interface_options.get(index).cloned() else {
            return;
        };
        let wifi_targets = option
            .wifi_id
            .as_ref()
            .and_then(|id| self.wifi_networks.get(id))
            .cloned()
            .unwrap_or_default();
        let height = if option.wifi_id.is_some() { 370 } else { 240 };
        let dialog = self.modal_window(owner, &format!("接口详情 · {}", option.name), 500, height);
        self.modal_control(
            dialog,
            "STATIC",
            "自定义网关（留空使用默认网关）",
            0,
            24,
            22,
            440,
            22,
            0,
        );
        let gateway = self.modal_control(
            dialog,
            "EDIT",
            &self
                .interface_gateways
                .get(&option.binding)
                .map(ToString::to_string)
                .unwrap_or_default(),
            ES_AUTOHSCROLL as u32 | WS_TABSTOP,
            24,
            48,
            270,
            30,
            9101,
        );
        let mut default_wifi = null_mut();
        let mut free_wifi = null_mut();
        let mut custom_enabled = null_mut();
        let mut custom_name = null_mut();
        if option.wifi_id.is_some() {
            self.modal_control(dialog, "STATIC", "Wi-Fi 自动连接", 0, 24, 96, 300, 22, 0);
            default_wifi = self.modal_control(
                dialog,
                "BUTTON",
                "BUAA-WiFi",
                BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
                24,
                126,
                220,
                24,
                9102,
            );
            free_wifi = self.modal_control(
                dialog,
                "BUTTON",
                "BUAA-WiFi-Free",
                BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
                24,
                158,
                220,
                24,
                9103,
            );
            custom_enabled = self.modal_control(
                dialog,
                "BUTTON",
                "自定义 Wi-Fi 名称",
                BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
                24,
                190,
                200,
                24,
                9104,
            );
            let custom = wifi_targets
                .iter()
                .find(|ssid| *ssid != "BUAA-WiFi" && *ssid != "BUAA-WiFi-Free")
                .cloned()
                .unwrap_or_default();
            custom_name = self.modal_control(
                dialog,
                "EDIT",
                &custom,
                ES_AUTOHSCROLL as u32 | WS_TABSTOP,
                230,
                188,
                234,
                30,
                9105,
            );
            for (control, checked) in [
                (default_wifi, wifi_targets.contains("BUAA-WiFi")),
                (free_wifi, wifi_targets.contains("BUAA-WiFi-Free")),
                (custom_enabled, !custom.is_empty()),
            ] {
                SendMessageW(
                    control,
                    BM_SETCHECK,
                    if checked { BST_CHECKED } else { BST_UNCHECKED } as usize,
                    0,
                );
            }
            self.modal_control(
                dialog,
                "STATIC",
                "校园网网页密码不会用于 Wi-Fi 连接。",
                0,
                24,
                232,
                440,
                36,
                0,
            );
        }
        let button_y = height - 74;
        self.modal_control(
            dialog, "BUTTON", "取消", WS_TABSTOP, 270, button_y, 90, 32, 2,
        );
        self.modal_control(
            dialog,
            "BUTTON",
            "保存",
            BS_DEFPUSHBUTTON as u32 | WS_TABSTOP,
            372,
            button_y,
            92,
            32,
            1,
        );
        EnableWindow(owner, 0);
        ShowWindow(dialog, SW_SHOW);
        SetForegroundWindow(dialog);
        let mut message: MSG = std::mem::zeroed();
        'messages: while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            if message.hwnd == dialog && message.message == MODAL_CLOSE {
                break;
            }
            if message.hwnd == dialog && message.message == MODAL_COMMAND {
                match message.wParam & 0xffff {
                    2 => break 'messages,
                    1 => {
                        let value = text(gateway).trim().to_owned();
                        if value.is_empty() {
                            self.interface_gateways.remove(&option.binding);
                        } else if let Ok(value) = value.parse() {
                            self.interface_gateways
                                .insert(option.binding.clone(), value);
                        } else {
                            MessageBoxW(
                                dialog,
                                wide("网关必须是 IPv4 地址").as_ptr(),
                                wide("接口详情").as_ptr(),
                                MB_OK | MB_ICONINFORMATION,
                            );
                            continue;
                        }
                        if let Some(wifi_id) = &option.wifi_id {
                            let mut targets = HashSet::new();
                            if SendMessageW(default_wifi, BM_GETCHECK, 0, 0) == BST_CHECKED as isize
                            {
                                targets.insert("BUAA-WiFi".into());
                            }
                            if SendMessageW(free_wifi, BM_GETCHECK, 0, 0) == BST_CHECKED as isize {
                                targets.insert("BUAA-WiFi-Free".into());
                            }
                            let custom = text(custom_name).trim().to_owned();
                            if SendMessageW(custom_enabled, BM_GETCHECK, 0, 0)
                                == BST_CHECKED as isize
                                && !custom.is_empty()
                            {
                                targets.insert(custom);
                            }
                            if targets.is_empty() {
                                self.wifi_networks.remove(wifi_id);
                            } else {
                                self.wifi_networks.insert(wifi_id.clone(), targets);
                            }
                            let _ = self.persist_wifi_networks();
                        }
                        break 'messages;
                    }
                    _ => {}
                }
            }
            if IsDialogMessageW(dialog, &message) == 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        DestroyWindow(dialog);
        EnableWindow(owner, 1);
        SetForegroundWindow(owner);
    }
    unsafe fn show_interface_dialog(&mut self) {
        if self.handle.is_some() {
            MessageBoxW(
                self.window,
                wide("请先停止自动连接再修改接口。").as_ptr(),
                wide("接口设置").as_ptr(),
                MB_OK | MB_ICONINFORMATION,
            );
            return;
        }
        self.refresh_wifi_options();
        let rows_height = (self.interface_options.len() as i32 * 38).max(42);
        let form_y = 68 + rows_height;
        let dialog_height = form_y + 174;
        let dialog = self.modal_window(self.window, "接口设置", 600, dialog_height);
        self.modal_control(
            dialog,
            "STATIC",
            "校园网接口（可多选）",
            0,
            24,
            20,
            340,
            24,
            0,
        );
        self.modal_control(
            dialog,
            "BUTTON",
            "检测校园网",
            WS_TABSTOP,
            420,
            16,
            154,
            32,
            3,
        );
        let mut checks = Vec::new();
        for (index, option) in self.interface_options.iter().enumerate() {
            let kind = if option.wifi_id.is_some() {
                "Wi-Fi"
            } else {
                "有线"
            };
            let check = self.modal_control(
                dialog,
                "BUTTON",
                &format!("{} · {} · {}", kind, option.label, option.message),
                BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
                24,
                58 + index as i32 * 38,
                462,
                28,
                9400 + index,
            );
            if self.selected_bindings.contains(&option.binding) {
                SendMessageW(check, BM_SETCHECK, BST_CHECKED as usize, 0);
            }
            checks.push(check);
            self.modal_control(
                dialog,
                "BUTTON",
                "详情…",
                WS_TABSTOP,
                500,
                56 + index as i32 * 38,
                74,
                30,
                9300 + index,
            );
        }
        if self.interface_options.is_empty() {
            self.modal_control(
                dialog,
                "STATIC",
                "尚未检测接口，请点击“检测校园网”。",
                0,
                24,
                60,
                500,
                24,
                0,
            );
        }
        self.modal_control(
            dialog,
            "STATIC",
            "未选择时自动选择；逐接口网关和 Wi-Fi 自动连接请进入详情。",
            0,
            24,
            form_y,
            550,
            22,
            0,
        );
        self.modal_control(
            dialog,
            "STATIC",
            "默认网关地址",
            0,
            24,
            form_y + 38,
            140,
            22,
            0,
        );
        let gateway = self.modal_control(
            dialog,
            "EDIT",
            &text(self.gateway),
            ES_AUTOHSCROLL as u32 | WS_TABSTOP,
            170,
            form_y + 32,
            220,
            30,
            9202,
        );
        self.modal_control(
            dialog,
            "BUTTON",
            "取消",
            WS_TABSTOP,
            370,
            form_y + 88,
            90,
            32,
            2,
        );
        self.modal_control(
            dialog,
            "BUTTON",
            "完成",
            BS_DEFPUSHBUTTON as u32 | WS_TABSTOP,
            474,
            form_y + 88,
            100,
            32,
            1,
        );
        EnableWindow(self.window, 0);
        ShowWindow(dialog, SW_SHOW);
        SetForegroundWindow(dialog);
        let mut run_scan = false;
        let mut message: MSG = std::mem::zeroed();
        'messages: while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            if message.hwnd == dialog && message.message == MODAL_CLOSE {
                break;
            }
            if message.hwnd == dialog && message.message == MODAL_COMMAND {
                match message.wParam & 0xffff {
                    2 => break 'messages,
                    3 => {
                        run_scan = true;
                        break 'messages;
                    }
                    1 => {
                        let value = text(gateway).trim().to_owned();
                        if value.parse::<Ipv4Addr>().is_err() {
                            MessageBoxW(
                                dialog,
                                wide("默认网关必须是 IPv4 地址").as_ptr(),
                                wide("接口设置").as_ptr(),
                                MB_OK | MB_ICONINFORMATION,
                            );
                            continue;
                        }
                        SetWindowTextW(self.gateway, wide(&value).as_ptr());
                        self.selected_bindings.clear();
                        for (check, option) in checks.iter().zip(&self.interface_options) {
                            if SendMessageW(*check, BM_GETCHECK, 0, 0) == BST_CHECKED as isize {
                                self.selected_bindings.insert(option.binding.clone());
                            }
                        }
                        if let Some(config) = self.configuration(false) {
                            let _ = self.save_preferences(&config);
                        }
                        break 'messages;
                    }
                    id if (9300..9300 + self.interface_options.len()).contains(&id) => {
                        self.show_interface_detail_dialog(dialog, id - 9300);
                    }
                    _ => {}
                }
            }
            if IsDialogMessageW(dialog, &message) == 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        DestroyWindow(dialog);
        EnableWindow(self.window, 1);
        SetForegroundWindow(self.window);
        if run_scan {
            self.start_worker(true);
        }
    }
    unsafe fn persist_wifi_networks(&self) -> Result<(), String> {
        let path = wifi_path()?;
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|_| "无法创建 Wi-Fi 配置目录")?;
        atomic_write(&path, &serde_json::to_vec(&self.wifi_networks).unwrap())?;
        if let Some(wifi) = &self.wifi {
            wifi.configure(
                self.wifi_networks.clone(),
                self.handle.is_some() && !self.scanning,
            );
        }
        Ok(())
    }
    unsafe fn save_preferences(&self, config: &Config) -> Result<(), String> {
        let path = credential_path()?;
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|_| "无法创建配置目录")?;
        let settings = serde_json::json!({
            "interval": config.interval,
            "gateway": config.gateway_ip.to_string(),
            "interfaces": config.interfaces,
            "interface_gateways": config.interface_gateways.iter().map(|(key, value)| (key, value.to_string())).collect::<HashMap<_, _>>(),
            "portal_auth_enabled": config.portal_auth_enabled,
            "connectivity_probe_enabled": config.connectivity_probe_enabled,
            "browser_fallback": config.browser_fallback,
            "browser_path": config.browser_path,
            "traffic_total_gb": text(self.traffic_total).trim(),
        });
        atomic_write(
            &path.with_extension("settings.json"),
            &serde_json::to_vec(&settings).unwrap(),
        )
    }
    unsafe fn save_general_settings(&mut self) {
        let visible_interval = GetDlgItem(self.settings_pane, 51);
        let interval = text(visible_interval).trim().parse::<u64>().ok();
        let total = text(self.traffic_total).trim().to_owned();
        if interval.is_none() || !(10..=86400).contains(&interval.unwrap_or_default()) {
            MessageBoxW(
                self.window,
                wide("检查周期须为 10–86400 的整数").as_ptr(),
                wide("设置").as_ptr(),
                MB_OK | MB_ICONINFORMATION,
            );
            return;
        }
        if !total.is_empty()
            && total
                .parse::<f64>()
                .ok()
                .is_none_or(|value| !value.is_finite() || value <= 0.0)
        {
            MessageBoxW(
                self.window,
                wide("总流量请输入大于 0 的数字，或留空关闭百分比。").as_ptr(),
                wide("设置").as_ptr(),
                MB_OK | MB_ICONINFORMATION,
            );
            return;
        }
        SetWindowTextW(self.interval, wide(&interval.unwrap().to_string()).as_ptr());
        if let Some(handle) = &self.handle {
            if let Err(error) = handle.signal.set_interval(interval.unwrap()) {
                MessageBoxW(
                    self.window,
                    wide(&error).as_ptr(),
                    wide("设置").as_ptr(),
                    MB_OK | MB_ICONINFORMATION,
                );
                return;
            }
        }
        if let Some(config) = self.configuration(false) {
            match self.save_preferences(&config) {
                Ok(()) => self.record("设置已保存"),
                Err(error) => self.record(&error),
            }
        }
        self.update_start_title();
        self.update_traffic();
    }
    unsafe fn update_traffic(&self) {
        let total = text(self.traffic_total)
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && *value > 0.0)
            .map(|value| value * 1_000_000_000.0);
        let (label, progress) = match self.traffic_used_bytes {
            Some(used) => {
                let used_label = format_bytes(used);
                match total {
                    Some(total) => (
                        format!("已用 {used_label} / {} GB", text(self.traffic_total).trim()),
                        ((used as f64 / total).min(1.0) * 1000.0) as usize,
                    ),
                    None => (format!("已用 {used_label}"), 0),
                }
            }
            None => ("尚未获取流量".into(), 0),
        };
        SetWindowTextW(self.traffic_label, wide(&label).as_ptr());
        SendMessageW(self.traffic_progress, PBM_SETPOS, progress, 0);
    }
    unsafe fn save_credentials(&self, config: &Config) -> Result<(), String> {
        let path = credential_path()?;
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|_| "无法创建配置目录")?;
        self.save_preferences(config)?;
        if SendMessageW(self.remember, BM_GETCHECK, 0, 0) == BST_CHECKED as isize {
            let mut bytes = serde_json::to_vec(&serde_json::json!({"username":config.username,"password":config.password.as_str()})).unwrap();
            let encrypted = crate::credentials::encrypt(&bytes);
            bytes.fill(0);
            atomic_write(&path, &encrypted)?;
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
                    SetWindowTextW(
                        GetDlgItem(self.settings_pane, 51),
                        wide(&n.to_string()).as_ptr(),
                    );
                }
                if let Some(g) = v["gateway"].as_str() {
                    SetWindowTextW(self.gateway, wide(g).as_ptr());
                }
                SendMessageW(
                    self.browser_fallback,
                    BM_SETCHECK,
                    if v["browser_fallback"].as_bool().unwrap_or(true) {
                        BST_CHECKED
                    } else {
                        BST_UNCHECKED
                    } as usize,
                    0,
                );
                SendMessageW(
                    self.portal_auth,
                    BM_SETCHECK,
                    if v["portal_auth_enabled"].as_bool().unwrap_or(true) {
                        BST_CHECKED
                    } else {
                        BST_UNCHECKED
                    } as usize,
                    0,
                );
                SendMessageW(
                    self.connectivity_probe,
                    BM_SETCHECK,
                    if v["connectivity_probe_enabled"].as_bool().unwrap_or(true) {
                        BST_CHECKED
                    } else {
                        BST_UNCHECKED
                    } as usize,
                    0,
                );
                SetWindowTextW(
                    self.browser_path,
                    wide(v["browser_path"].as_str().unwrap_or("")).as_ptr(),
                );
                SetWindowTextW(
                    self.traffic_total,
                    wide(v["traffic_total_gb"].as_str().unwrap_or("")).as_ptr(),
                );
                self.selected_bindings = v["interfaces"]
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.as_str().map(str::to_owned))
                            .collect()
                    })
                    .or_else(|| {
                        v["interface"]
                            .as_str()
                            .map(|item| HashSet::from([item.to_owned()]))
                    })
                    .unwrap_or_default();
                self.interface_gateways = v["interface_gateways"]
                    .as_object()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|(key, value)| {
                                value
                                    .as_str()?
                                    .parse()
                                    .ok()
                                    .map(|value| (key.clone(), value))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
            }
        }
        if let Ok(bytes) = std::fs::read(&path) {
            match crate::credentials::decrypt(&bytes) {
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
                Err(_) => self.record("应用内凭据无法解密，请重新输入"),
            }
        }
        self.update_traffic();
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
fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}
fn credential_path() -> Result<PathBuf, String> {
    Ok(
        PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("用户目录不可用")?)
            .join("BUAALoginNative/v1/credentials.dat"),
    )
}
fn log_folder() -> Result<PathBuf, String> {
    Ok(
        PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("用户目录不可用")?)
            .join("BUAALoginNative/Logs"),
    )
}
fn append_log(line: &str) -> Result<(), String> {
    use std::io::Write;
    let folder = log_folder()?;
    std::fs::create_dir_all(&folder).map_err(|_| "无法创建日志目录")?;
    let mut local: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe {
        GetLocalTime(&mut local);
    }
    let path = folder.join(format!(
        "BUAALogin-{:04}-{:02}-{:02}.log",
        local.wYear, local.wMonth, local.wDay
    ));
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|_| "无法写入日志")?;
    writeln!(file, "{line}").map_err(|_| "无法写入日志".into())
}
fn cleanup_logs() {
    let Ok(folder) = log_folder() else {
        return;
    };
    let _ = std::fs::create_dir_all(&folder);
    let cutoff =
        std::time::SystemTime::now().checked_sub(std::time::Duration::from_secs(30 * 86400));
    if let (Some(cutoff), Ok(entries)) = (cutoff, std::fs::read_dir(folder)) {
        for entry in entries.flatten() {
            if entry
                .metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|m| m < cutoff)
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
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
unsafe extern "system" fn modal_proc(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    _data: usize,
) -> LRESULT {
    match message {
        WM_COMMAND => {
            if w >> 16 == BN_CLICKED as usize {
                PostMessageW(hwnd, MODAL_COMMAND, w & 0xffff, 0);
            }
            0
        }
        WM_CLOSE => {
            PostMessageW(hwnd, MODAL_CLOSE, 0, 0);
            0
        }
        WM_NCDESTROY => {
            RemoveWindowSubclass(hwnd, Some(modal_proc), id);
            DefSubclassProc(hwnd, message, w, l)
        }
        _ => DefSubclassProc(hwnd, message, w, l),
    }
}
unsafe extern "system" fn navigation_proc(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    if let Some(app) = (data as *const App).as_ref() {
        match message {
            WM_PAINT => {
                app.paint_navigation(hwnd);
                return 0;
            }
            WM_ERASEBKGND => return 1,
            WM_SETFOCUS | WM_KILLFOCUS | WM_UPDATEUISTATE => {
                InvalidateRect(hwnd, null(), 0);
            }
            WM_NCDESTROY => {
                RemoveWindowSubclass(hwnd, Some(navigation_proc), id);
            }
            _ => {}
        }
    }
    DefSubclassProc(hwnd, message, w, l)
}
unsafe extern "system" fn settings_proc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if message == WM_NCCREATE {
        let create = &*(l as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    let app = (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App).as_mut();
    if let Some(app) = app {
        match message {
            WM_PAINT => {
                app.paint(hwnd, true);
                return 0;
            }
            WM_ERASEBKGND => return 1,
            WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORBTN | WM_CTLCOLORLISTBOX
            | WM_DRAWITEM => {
                return SendMessageW(app.window, message, w, l);
            }
            WM_COMMAND => {
                let notification = (w >> 16) as u32;
                if notification == EN_SETFOCUS || notification == EN_KILLFOCUS {
                    InvalidateRect(hwnd, null(), 0);
                }
                if notification == BN_SETFOCUS {
                    app.ensure_setting_visible(l as HWND);
                }
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
            WM_PAINT => {
                app.paint(hwnd, false);
                return 0;
            }
            WM_ERASEBKGND => return 1,
            WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
                return app.control_color(w as HDC, l as HWND, false);
            }
            WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => {
                return app.control_color(w as HDC, l as HWND, true);
            }
            WM_DRAWITEM => {
                app.draw_item(&*(l as *const DRAWITEMSTRUCT));
                return 1;
            }
            WM_SYSCOLORCHANGE | WM_THEMECHANGED | WM_SETTINGCHANGE => {
                app.palette = Palette::system();
                RedrawWindow(hwnd, null(), null_mut(), RDW_INVALIDATE | RDW_ALLCHILDREN);
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
                let id = START;
                return ((DC_HASDEFID as usize) << 16 | id) as isize;
            }
            WM_COMMAND => {
                let id = w & 0xffff;
                // Owner-draw/BS_NOTIFY buttons also send focus and double-click notices.
                // Only a click may start work or mutate settings.
                if l != 0
                    && [
                        START,
                        STOP,
                        SCAN,
                        HIDE,
                        REMEMBER,
                        AUTOSTART,
                        REPAIR,
                        WEB_REPAIR,
                        BROWSER_FALLBACK,
                        PORTAL_AUTH,
                        CONNECTIVITY_PROBE,
                        TEST_PORTAL,
                        TEST_CONNECTIVITY,
                        TEST_BROWSER,
                        OPEN_LOGS,
                        CONNECTION_SETTINGS,
                        ACCOUNT_SETTINGS,
                        SAVE_SETTINGS,
                    ]
                    .contains(&id)
                    && w >> 16 != BN_CLICKED as usize
                {
                    return 0;
                }
                match w & 0xffff {
                    START => app.dashboard_action(),
                    OPEN_LOGS => app.open_log_folder(),
                    CONNECTION_SETTINGS => app.show_interface_dialog(),
                    ACCOUNT_SETTINGS => app.show_account_dialog(),
                    SAVE_SETTINGS => app.save_general_settings(),
                    AUTOSTART => app.startup_changed(),
                    20..=22 if w >> 16 == EN_CHANGE as usize => app.update_start_title(),
                    STOP => app.stop_worker(),
                    REPAIR => app.request_recovery(RecoveryMode::Protocol),
                    WEB_REPAIR => app.request_recovery(RecoveryMode::Browser),
                    TEST_PORTAL => app.run_test(TestKind::PortalAuth),
                    TEST_CONNECTIVITY => app.run_test(TestKind::Connectivity),
                    TEST_BROWSER => app.run_test(TestKind::Browser),
                    CONNECTIVITY_PROBE => app.connectivity_changed(),
                    PORTAL_AUTH => {
                        SetWindowTextW(
                            app.task_next[0],
                            wide(
                                if SendMessageW(app.portal_auth, BM_GETCHECK, 0, 0)
                                    == BST_CHECKED as isize
                                {
                                    "启动后立即"
                                } else {
                                    "已关闭"
                                },
                            )
                            .as_ptr(),
                        );
                        app.update_start_title();
                    }
                    BROWSER_FALLBACK => app.connectivity_changed(),
                    SCAN => app.start_worker(true),
                    HIDE => {
                        ShowWindow(hwnd, SW_HIDE);
                    }
                    SHOW => app.show(),
                    QUIT => app.quit(),
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
        let modal_name = wide("BUAALoginModalWindow");
        let modal_class = WNDCLASSW {
            lpfnWndProc: Some(DefWindowProcW),
            hInstance: instance,
            lpszClassName: modal_name.as_ptr(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
            ..std::mem::zeroed()
        };
        RegisterClassW(&modal_class);
        let mut app = Box::new(App::new());
        let dpi = system_dpi();
        let style =
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_VSCROLL | WS_CLIPCHILDREN;
        let mut frame = RECT {
            left: 0,
            top: 0,
            right: 680 * dpi / 96,
            bottom: 700 * dpi / 96,
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
                    app.show_account_dialog();
                }
            } else {
                ShowWindow(window, SW_SHOW);
                app.show_account_dialog();
                app.record("开机自启：请保存完整账号、密码和连接配置后启用自动连接。");
            }
        } else {
            ShowWindow(window, SW_SHOW);
            app.start_worker(true);
        }
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            // IsDialogMessage does not recognise an owner-draw button as a default
            // pushbutton. Enter must activate the focused action, not another page action.
            if message.message == WM_KEYDOWN && message.wParam == 13 {
                let focus = GetFocus();
                let parent = GetParent(focus);
                let owner_draw = GetWindowLongPtrW(focus, GWL_STYLE) as u32 & BS_TYPEMASK as u32
                    == BS_OWNERDRAW as u32;
                if (parent == window || parent == app.settings_pane)
                    && owner_draw
                    && windows_sys::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(focus) != 0
                {
                    SendMessageW(focus, BM_CLICK, 0, 0);
                    continue;
                }
            }
            if IsDialogMessageW(window, &message) == 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
}
