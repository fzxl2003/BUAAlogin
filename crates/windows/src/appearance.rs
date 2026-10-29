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
    pub error: COLORREF,
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
                error: GetSysColor(COLOR_WINDOWTEXT),
                high_contrast: true,
            };
        }
        Self {
            background: rgb(245, 247, 249),
            surface: rgb(255, 255, 255),
            text: rgb(28, 39, 59),
            muted: rgb(96, 111, 132),
            border: rgb(223, 229, 238),
            accent: rgb(0, 120, 240),
            accent_text: rgb(255, 255, 255),
            soft: rgb(233, 239, 250),
            success: rgb(36, 184, 88),
            warning: rgb(214, 137, 24),
            error: rgb(211, 57, 63),
            high_contrast: false,
        }
    }
}

/// Only known result labels get a dot. Test/running messages retain plain text,
/// so updating the native label also clears a previous success indication.
pub fn badge_color(label: &[u16], palette: Palette) -> Option<COLORREF> {
    for (text, color) in [
        ("已认证", palette.success),
        ("可用", palette.success),
        ("未认证", palette.warning),
        ("不可用", palette.error),
        ("未知", palette.muted),
    ] {
        if label.iter().copied().eq(text.encode_utf16()) {
            return Some(color);
        }
    }
    None
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badges_distinguish_authentication_connectivity_and_unknown() {
        let p = unsafe { Palette::system() };
        for (label, expected) in [
            ("已认证", p.success),
            ("可用", p.success),
            ("未认证", p.warning),
            ("不可用", p.error),
            ("未知", p.muted),
        ] {
            assert_eq!(
                badge_color(&label.encode_utf16().collect::<Vec<_>>(), p),
                Some(expected)
            );
        }
    }

    #[test]
    fn testing_and_fallback_messages_do_not_keep_a_success_dot() {
        let p = unsafe { Palette::system() };
        for label in ["正在测试…", "尚未运行", "—", "测试失败", "认证测试成功"]
        {
            assert_eq!(
                badge_color(&label.encode_utf16().collect::<Vec<_>>(), p),
                None
            );
        }
    }
}
