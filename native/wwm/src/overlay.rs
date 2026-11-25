//! Workspace overlay: a topmost, click-through dark panel with the current
//! workspace number. Shown for ~1.1s on Win+Space.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, HINSTANCE, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateFontW, CreateSolidBrush, GetMonitorInfoW, MonitorFromPoint, SetBkMode, SetTextColor,
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, HBRUSH, HDC, HFONT, MONITORINFO,
    MONITOR_DEFAULTTONEAREST, OUT_DEFAULT_PRECIS, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetForegroundWindow, GetWindowRect, RegisterClassW,
    SendMessageW, SetWindowPos, SetWindowTextW, ShowWindow, WNDCLASSW, CW_USEDEFAULT,
    HWND_TOPMOST, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_CTLCOLORSTATIC, WM_SETFONT, WS_CHILD, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};

const OVERLAY_W: i32 = 390;
const OVERLAY_H: i32 = 92;
const OVERLAY_TOP: i32 = 42;

thread_local! {
    static DARK_BRUSH: RefCell<Option<HBRUSH>> = const { RefCell::new(None) };
    static BIG_FONT: RefCell<Option<HFONT>> = const { RefCell::new(None) };
}

unsafe extern "system" fn overlay_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_CTLCOLORSTATIC {
        let dc = HDC(wparam.0 as *mut core::ffi::c_void);
        let brush = DARK_BRUSH.with(|b| b.borrow().as_ref().copied());
        if let Some(brush) = brush {
            let _ = SetTextColor(dc, COLORREF(0x00F8FAFC)); // near-white
            let _ = SetBkMode(dc, TRANSPARENT);
            return LRESULT(brush.0 as isize);
        }
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

pub struct Overlay {
    hwnd: HWND,
    label: HWND,
    duration: Duration,
    shown_at: Option<Instant>,
}

impl Overlay {
    pub fn new(duration_ms: u64) -> Option<Self> {
        unsafe {
            let module = GetModuleHandleW(None).ok()?;
            let class = w!("WWMOverlay");
            let brush = CreateSolidBrush(COLORREF(0x002A170F)); // #0F172A
            DARK_BRUSH.with(|b| *b.borrow_mut() = Some(brush));

            let wc = WNDCLASSW {
                lpfnWndProc: Some(overlay_wndproc),
                hInstance: HINSTANCE(module.0),
                hbrBackground: brush,
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassW(&wc);

            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                class,
                w!(""),
                WS_POPUP,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                OVERLAY_W,
                OVERLAY_H,
                None,
                None,
                Some(HINSTANCE(module.0)),
                None,
            )
            .ok()?;

            // SS_CENTER = 0x1.
            let label = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!(""),
                WINDOW_STYLE(0x1 | WS_CHILD.0 as u32 | WS_VISIBLE.0 as u32),
                8,
                18,
                OVERLAY_W - 16,
                OVERLAY_H - 36,
                Some(hwnd),
                None,
                Some(HINSTANCE(module.0)),
                None,
            )
            .ok()?;

            let font = CreateFontW(
                -28,       // height
                0,         // width
                0,         // escapement
                0,         // orientation
                700,       // weight: bold
                0,         // italic
                0,         // underline
                0,         // strikeout
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,         // pitch + family
                w!("Segoe UI"),
            );
            BIG_FONT.with(|f| *f.borrow_mut() = Some(font));
            let _ = SendMessageW(
                label,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );

            Some(Self {
                hwnd,
                label,
                duration: Duration::from_millis(duration_ms),
                shown_at: None,
            })
        }
    }

    pub fn show(&mut self, text: &str) {
        let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        unsafe {
            let _ = SetWindowTextW(self.label, PCWSTR(wide.as_ptr()));
            self.position();
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        }
        self.shown_at = Some(Instant::now());
    }

    pub fn tick(&mut self) {
        if let Some(t) = self.shown_at {
            if t.elapsed() >= self.duration {
                self.hide();
            }
        }
    }

    pub fn hide(&mut self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        self.shown_at = None;
    }

    fn position(&self) {
        unsafe {
            let fg = GetForegroundWindow();
            let mut rect = RECT::default();
            if fg.0.is_null() || GetWindowRect(fg, &mut rect).is_err() {
                return;
            }
            let cx = rect.left + (rect.right - rect.left) / 2;
            let cy = rect.top + (rect.bottom - rect.top) / 2;
            let monitor = MonitorFromPoint(POINT { x: cx, y: cy }, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO::default();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(monitor, &mut mi).as_bool() {
                let work = mi.rcWork;
                let x = work.left + ((work.right - work.left) - OVERLAY_W) / 2;
                let y = work.top + OVERLAY_TOP;
                let _ = SetWindowPos(
                    self.hwnd,
                    Some(HWND_TOPMOST),
                    x,
                    y,
                    OVERLAY_W,
                    OVERLAY_H,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
            }
        }
    }
}
