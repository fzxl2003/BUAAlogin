use crate::appearance::{self, Palette};
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
    stop: HWND,
    scan: HWND,
    status: HWND,
    history: HWND,
    tabs: HWND,
    page_controls: RefCell<Vec<(HWND, usize)>>,
    building_page: Cell<Option<usize>>,
    building_advanced: Cell<bool>,
    advanced_controls: RefCell<Vec<HWND>>,
    settings_positions: RefCell<Vec<ControlPosition>>,
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
    palette: Palette,
    header_icon: HICON,
    state_kind: Cell<u8>,
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
            palette: unsafe { Palette::system() },
            header_icon: null_mut(),
            state_kind: Cell::new(0),
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
                (self.username, 170, 82, 394, 30),
                (self.password, 170, 126, 394, 30),
                (self.interval, 170, 356, 86, 30),
                (self.gateway, 170, 454, 220, 30),
            ] {
                if control != self.gateway || self.advanced_expanded {
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
            }
            let y = self.px(268) - self.settings_scroll;
            appearance::line(dc, self.px(20), y, self.px(564), y, p.border);
            if self.advanced_expanded {
                let y = self.px(442) - self.settings_scroll;
                appearance::line(dc, self.px(20), y, self.px(564), y, p.border);
            }
        } else if self.active_page == 0 {
            for (y, height) in [(80, 98), (188, 166), (404, 100)] {
                appearance::rounded(
                    dc,
                    &self.rect(32, y, 616, height, self.scroll),
                    self.px(16),
                    p.surface,
                    p.border,
                );
            }
            let x = self.px(340);
            appearance::line(
                dc,
                x,
                self.px(204) - self.scroll,
                x,
                self.px(338) - self.scroll,
                p.border,
            );
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
            || (self.active_page == 0 && y >= self.px(80) && y < self.px(354));
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
            let primary = item.CtlID as usize == START || item.CtlID as usize == SETTINGS_START;
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
                let label = if item.itemID == 0 {
                    "连接仪表盘"
                } else {
                    "设置"
                };
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
        for (index, title) in ["连接仪表盘", "设置"].iter().enumerate() {
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
            self.px(424),
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
            "先在设置中填写账号，再开始自动重连。",
            0,
            120,
            135,
            504,
            36,
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
            let x = 52 + (index as i32 % 2) * 304;
            let y = 201 + (index as i32 / 2) * 54;
            self.label(title, x, y, 260);
            self.dashboard_values[index] =
                self.control("STATIC", "—", SS_ENDELLIPSIS, x, y + 23, 260, 24, 0);
            SendMessageW(
                self.dashboard_values[index],
                WM_SETFONT,
                self.heading_font as usize,
                1,
            );
        }
        let recent = self.label("最近活动", 32, 373, 300);
        SendMessageW(recent, WM_SETFONT, self.heading_font as usize, 1);
        self.history_button = self.control(
            "BUTTON",
            "查看完整记录",
            WS_TABSTOP,
            492,
            368,
            156,
            30,
            HISTORY,
        );
        self.history = self.control(
            "EDIT",
            "暂无活动记录",
            ES_MULTILINE as u32
                | ES_READONLY as u32
                | ES_AUTOVSCROLL as u32
                | WS_VSCROLL
                | WS_TABSTOP,
            46,
            415,
            588,
            74,
            0,
        );
        self.start = self.control(
            "BUTTON",
            "设置账号",
            BS_DEFPUSHBUTTON as u32 | WS_TABSTOP,
            32,
            530,
            184,
            38,
            START,
        );
        self.stop = self.control("BUTTON", "停止", WS_TABSTOP, 228, 530, 88, 38, STOP);
        EnableWindow(self.stop, 0);
        self.control("BUTTON", "收起至托盘", WS_TABSTOP, 492, 530, 156, 38, HIDE);

        self.building_page.set(Some(1));
        let title = self.label("账号与启动", 20, 14, 560);
        SendMessageW(title, WM_SETFONT, self.heading_font as usize, 1);
        self.label("使用校园网账号登录，密码由 Windows 加密保存。", 20, 44, 560);
        self.label("校园网账号", 20, 89, 140);
        self.username = self.control(
            "EDIT",
            "",
            WS_BORDER | ES_AUTOHSCROLL as u32 | WS_TABSTOP,
            170,
            82,
            394,
            30,
            20,
        );
        SendMessageW(
            self.username,
            EM_SETCUEBANNER,
            0,
            wide("学号 / 工号").as_ptr() as isize,
        );
        self.label("校园网密码", 20, 133, 140);
        self.password = self.control(
            "EDIT",
            "",
            WS_BORDER | ES_PASSWORD as u32 | ES_AUTOHSCROLL as u32 | WS_TABSTOP,
            170,
            126,
            394,
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
            170,
            170,
            394,
            24,
            REMEMBER,
        );
        self.startup = self.control(
            "BUTTON",
            "开机自启",
            BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
            170,
            204,
            394,
            24,
            AUTOSTART,
        );
        self.control(
            "STATIC",
            "登录系统后自动连接，需先记住账号密码。",
            0,
            170,
            236,
            394,
            22,
            0,
        );
        let title = self.label("连接", 20, 278, 550);
        SendMessageW(title, WM_SETFONT, self.heading_font as usize, 1);
        self.label("校园网接口", 20, 318, 140);
        self.interfaces = self.control(
            "COMBOBOX",
            "",
            CBS_DROPDOWNLIST as u32 | WS_VSCROLL | WS_TABSTOP,
            170,
            312,
            270,
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
        self.scan = self.control("BUTTON", "检测接口", WS_TABSTOP, 452, 310, 112, 30, SCAN);
        self.label("检查间隔", 20, 363, 140);
        self.interval = self.control(
            "EDIT",
            "300",
            WS_BORDER | ES_NUMBER as u32 | WS_TABSTOP,
            170,
            356,
            86,
            30,
            22,
        );
        self.label("秒 · 失败后逐步延长重试间隔", 272, 363, 292);
        self.control(
            "BUTTON",
            "展开高级选项",
            WS_TABSTOP,
            20,
            390,
            154,
            30,
            ADVANCED,
        );
        self.building_advanced.set(true);
        self.label("网关地址", 20, 460, 140);
        self.gateway = self.control(
            "EDIT",
            "10.200.21.4",
            WS_BORDER | WS_TABSTOP,
            170,
            454,
            220,
            30,
            24,
        );
        self.label("通常无需修改", 406, 460, 158);
        self.label("Wi-Fi 网卡", 20, 506, 140);
        self.wifi_adapter = self.control(
            "COMBOBOX",
            "",
            CBS_DROPDOWNLIST as u32 | WS_VSCROLL | WS_TABSTOP,
            170,
            500,
            394,
            260,
            WIFI_ADAPTER,
        );
        self.wifi_auto = self.control(
            "BUTTON",
            "发现 BUAA-WiFi 自动连接",
            BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
            170,
            546,
            394,
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
            462,
            530,
            186,
            38,
            SETTINGS_START,
        );
        self.settings_hint = self.label("开始时保存并应用设置。", 32, 539, 410);
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
            "online" => ("校园网在线", 1),
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
        SetWindowTextW(self.state_title, wide(title).as_ptr());
        InvalidateRect(self.state_icon, null(), 0);
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
        SetFocus(self.tabs);
        InvalidateRect(self.tabs, null(), 1);
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
            nMax: self.px(if self.advanced_expanded { 594 } else { 424 }) - 1,
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
                let id = if app.active_page == 0 {
                    START
                } else {
                    SETTINGS_START
                };
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
                        ADVANCED,
                        HISTORY,
                        SETTINGS_START,
                        AUTOSTART,
                        WIFI_AUTO,
                    ]
                    .contains(&id)
                    && w >> 16 != BN_CLICKED as usize
                {
                    return 0;
                }
                match w & 0xffff {
                    START => app.dashboard_action(),
                    SETTINGS_START => app.start_worker(false),
                    ADVANCED => app.toggle_advanced(),
                    HISTORY => app.toggle_history(),
                    AUTOSTART => app.startup_changed(),
                    20..=22 if w >> 16 == EN_CHANGE as usize => app.update_start_title(),
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
        let style =
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_VSCROLL | WS_CLIPCHILDREN;
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
