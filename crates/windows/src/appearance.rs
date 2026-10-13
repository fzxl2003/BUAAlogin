//! Small, event-driven GDI primitives. No textures, retained surfaces or timers.
use std::ptr::null_mut;
use windows_sys::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows_sys::Win32::{Foundation::*, Graphics::Gdi::*, UI::WindowsAndMessaging::*};

pub const fn rgb(r: u32, g: u32, b: u32) -> COLORREF {
    r | (g << 8) | (b << 16)
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub background: COLORREF,
    pub surface: COLORREF,
    pub text: COLORREF,
    pub muted: COLORREF,
    pub border: COLORREF,
    pub accent: COLORREF,
    pub accent_text: COLORREF,
    pub soft: COLORREF,
    pub success: COLORREF,
    pub warning: COLORREF,
    pub high_contrast: bool,
}
impl Palette {
    pub unsafe fn system() -> Self {
        let mut hc: HIGHCONTRASTW = std::mem::zeroed();
        hc.cbSize = std::mem::size_of_val(&hc) as u32;
        SystemParametersInfoW(SPI_GETHIGHCONTRAST, hc.cbSize, &mut hc as *mut _ as _, 0);
        if hc.dwFlags & HCF_HIGHCONTRASTON != 0 {
            return Self {
                background: GetSysColor(COLOR_WINDOW),
                surface: GetSysColor(COLOR_WINDOW),
                text: GetSysColor(COLOR_WINDOWTEXT),
                muted: GetSysColor(COLOR_WINDOWTEXT),
                border: GetSysColor(COLOR_WINDOWTEXT),
                accent: GetSysColor(COLOR_HIGHLIGHT),
                accent_text: GetSysColor(COLOR_HIGHLIGHTTEXT),
                soft: GetSysColor(COLOR_WINDOW),
                success: GetSysColor(COLOR_WINDOWTEXT),
                warning: GetSysColor(COLOR_WINDOWTEXT),
                high_contrast: true,
            };
        }
        Self {
            background: rgb(245, 247, 251),
            surface: rgb(255, 255, 255),
            text: rgb(28, 39, 59),
            muted: rgb(96, 111, 132),
            border: rgb(223, 229, 238),
            accent: rgb(40, 93, 210),
            accent_text: rgb(255, 255, 255),
            soft: rgb(233, 239, 250),
            success: rgb(26, 125, 87),
            warning: rgb(166, 98, 13),
            high_contrast: false,
        }
    }
}

pub unsafe fn brush(dc: HDC, color: COLORREF) -> HBRUSH {
    SetDCBrushColor(dc, color);
    GetStockObject(DC_BRUSH) as HBRUSH
}
pub unsafe fn fill(dc: HDC, rect: &RECT, color: COLORREF) {
    FillRect(dc, rect, brush(dc, color));
}
pub unsafe fn rounded(dc: HDC, rect: &RECT, radius: i32, color: COLORREF, border: COLORREF) {
    let old_brush = SelectObject(dc, brush(dc, color));
    let old_pen = SelectObject(dc, GetStockObject(DC_PEN));
    SetDCPenColor(dc, border);
    RoundRect(
        dc,
        rect.left,
        rect.top,
        rect.right,
        rect.bottom,
        radius,
        radius,
    );
    SelectObject(dc, old_pen);
    SelectObject(dc, old_brush);
}
pub unsafe fn line(dc: HDC, x1: i32, y1: i32, x2: i32, y2: i32, color: COLORREF) {
    let old = SelectObject(dc, GetStockObject(DC_PEN));
    SetDCPenColor(dc, color);
    MoveToEx(dc, x1, y1, null_mut());
    LineTo(dc, x2, y2);
    SelectObject(dc, old);
}

/// Tiny procedural icon: avoids loading Shell stock-icon resources for a single glyph.
pub unsafe fn app_icon() -> HICON {
    let screen = GetDC(null_mut());
    let dc = CreateCompatibleDC(screen);
    let bitmap = CreateCompatibleBitmap(screen, 32, 32);
    let mask = CreateBitmap(32, 32, 1, 1, [0u8; 128].as_ptr() as _);
    let old = SelectObject(dc, bitmap);
    fill(
        dc,
        &RECT {
            left: 0,
            top: 0,
            right: 32,
            bottom: 32,
        },
        rgb(40, 93, 210),
    );
    let pen = CreatePen(PS_SOLID, 2, rgb(255, 255, 255));
    let old_pen = SelectObject(dc, pen);
    let old_brush = SelectObject(dc, GetStockObject(NULL_BRUSH));
    Arc(dc, 5, 8, 27, 29, 27, 16, 5, 16);
    Arc(dc, 10, 14, 22, 27, 22, 20, 10, 20);
    Ellipse(dc, 15, 24, 18, 27);
    SelectObject(dc, old_brush);
    SelectObject(dc, old_pen);
    SelectObject(dc, old);
    let info = ICONINFO {
        fIcon: 1,
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: mask,
        hbmColor: bitmap,
    };
    let icon = CreateIconIndirect(&info);
    DeleteObject(pen);
    DeleteObject(bitmap);
    DeleteObject(mask);
    DeleteDC(dc);
    ReleaseDC(null_mut(), screen);
    icon
}
