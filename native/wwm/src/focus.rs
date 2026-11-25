//! Focus management: stop taskbar flashing and focus a window on the new
//! desktop after a switch. Uses the undocumented `IApplicationView::SetFocus`
//! COM trick inline (no FocusBridge.dll).

use std::ffi::c_void;

use windows::core::{Interface, GUID, HRESULT, IUnknown, IUnknown_Vtbl, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::System::Com::{
    CoAllowSetForegroundWindow, CoCreateInstance, CLSCTX_LOCAL_SERVER, IServiceProvider,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{keybd_event, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VK_MENU};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FlashWindowEx, GetForegroundWindow, GetWindowLongPtrW, GetWindowTextLengthW,
    GetWindowTextW, IsIconic, IsWindow, IsWindowVisible, SetForegroundWindow, SetWindowPos,
    FLASHWINFO, FLASHW_STOP, GWL_EXSTYLE, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_NOOWNERZORDER, WS_EX_TOOLWINDOW,
};

use crate::vda::VirtualDesktopAccessor;

const CLSID_IMMERSIVE_SHELL: GUID = GUID {
    data1: 0xC2F03A33,
    data2: 0x21F5,
    data3: 0x47FA,
    data4: [0xB4, 0xBB, 0x15, 0x63, 0x62, 0xA2, 0xF2, 0x39],
};

#[allow(non_snake_case)]
#[repr(C)]
pub struct IApplicationView_Vtbl {
    base: IUnknown_Vtbl,
    GetIids: unsafe extern "system" fn(*mut c_void, *mut u32, *mut *mut GUID) -> HRESULT,
    GetRuntimeClassName: unsafe extern "system" fn(*mut c_void, *mut windows::core::HSTRING) -> HRESULT,
    GetTrustLevel: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    SetFocus: unsafe extern "system" fn(*mut c_void) -> HRESULT,
}
windows::core::imp::define_interface!(
    IApplicationView,
    IApplicationView_Vtbl,
    0x372e1d3b_38d3_42e4_a15b_8ab2b178f513
);
windows::core::imp::interface_hierarchy!(IApplicationView, IUnknown);
#[allow(non_snake_case)]
impl IApplicationView {
    pub unsafe fn SetFocus(&self) -> HRESULT {
        (Interface::vtable(self).SetFocus)(Interface::as_raw(self))
    }
}

#[allow(non_snake_case)]
#[repr(C)]
pub struct IApplicationViewCollection_Vtbl {
    base: IUnknown_Vtbl,
    GetViews: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    GetViewsByZOrder: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    GetViewsByAppUserModelId:
        unsafe extern "system" fn(*mut c_void, PCWSTR, *mut *mut c_void) -> HRESULT,
    GetViewForHwnd: unsafe extern "system" fn(*mut c_void, HWND, *mut *mut c_void) -> HRESULT,
}
windows::core::imp::define_interface!(
    IApplicationViewCollection,
    IApplicationViewCollection_Vtbl,
    0x1841c6d7_4f9d_42c0_af41_8747538f10e5
);
windows::core::imp::interface_hierarchy!(IApplicationViewCollection, IUnknown);
#[allow(non_snake_case)]
impl IApplicationViewCollection {
    pub unsafe fn GetViewForHwnd(&self, hwnd: HWND) -> windows::core::Result<IApplicationView> {
        let mut view: *mut c_void = std::ptr::null_mut();
        (Interface::vtable(self).GetViewForHwnd)(Interface::as_raw(self), hwnd, &mut view).ok()?;
        if view.is_null() {
            return Err(windows::core::Error::from_hresult(HRESULT(-2147467259))); // E_POINTER
        }
        Ok(IApplicationView::from_raw(view))
    }
}

struct FocusBridge {
    _provider: IServiceProvider,
    view: IApplicationView,
}

impl FocusBridge {
    fn prepare(hwnd: HWND) -> Option<Self> {
        unsafe {
            let provider: IServiceProvider =
                CoCreateInstance(&CLSID_IMMERSIVE_SHELL, None, CLSCTX_LOCAL_SERVER).ok()?;
            let collection: IApplicationViewCollection =
                provider.QueryService(&IApplicationViewCollection::IID).ok()?;
            let view = collection.GetViewForHwnd(hwnd).ok()?;

            let unknown: IUnknown = provider.cast().ok()?;
            let _ = CoAllowSetForegroundWindow(&unknown, None);
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOP),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            );

            Some(Self { _provider: provider, view })
        }
    }

    fn commit(&self) -> bool {
        unsafe { self.view.SetFocus().is_ok() }
    }
}

pub struct FocusManager {
    // Always available now that the COM is inlined.
    _private: (),
}

impl FocusManager {
    pub fn new() -> Self {
        Self { _private: () }
    }

    /// Stops the orange taskbar flash Windows starts when a foreground request
    /// fails or a window is stranded on another desktop.
    pub fn clear_all_flashes(&self) {
        let mut hwnds: Vec<HWND> = Vec::new();
        unsafe {
            let _ = EnumWindows(Some(enum_proc), LPARAM(&mut hwnds as *mut Vec<HWND> as isize));
        }
        for hwnd in hwnds {
            clear_flash(hwnd);
        }
    }

    /// Focuses a window belonging to the current desktop. Returns true once
    /// the foreground window is on the current desktop.
    pub fn focus_current_desktop(&self, vd: &VirtualDesktopAccessor) -> bool {
        let desktop = vd.current();
        let fg = unsafe { GetForegroundWindow() };
        if is_focusable(fg) && vd.window_desktop(fg.0 as isize) == desktop {
            return true;
        }

        let Some(target) = pick_target(vd, desktop) else {
            return false;
        };

        self.focus_window(target)
    }

    /// Focuses a specific window, using the COM bridge with a fallback.
    pub fn focus_window(&self, hwnd: HWND) -> bool {
        if let Some(bridge) = FocusBridge::prepare(hwnd) {
            if bridge.commit() {
                return true;
            }
        }

        // Fallback: release the foreground lock (classic Alt tap), then ask
        // Windows to focus the target directly.
        unsafe {
            keybd_event(VK_MENU.0 as u8, 0, KEYBD_EVENT_FLAGS(0), 0);
            keybd_event(VK_MENU.0 as u8, 0, KEYEVENTF_KEYUP, 0);
            let _ = SetForegroundWindow(hwnd);
        }
        unsafe { GetForegroundWindow().0 == hwnd.0 }
    }
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
    let windows = &mut *(lparam.0 as *mut Vec<HWND>);
    windows.push(hwnd);
    windows::core::BOOL(1)
}

fn clear_flash(hwnd: HWND) {
    let mut info = FLASHWINFO::default();
    info.cbSize = std::mem::size_of::<FLASHWINFO>() as u32;
    info.hwnd = hwnd;
    info.dwFlags = FLASHW_STOP;
    info.uCount = 0;
    info.dwTimeout = 0;
    unsafe {
        let _ = FlashWindowEx(&info);
    }
}

fn pick_target(vd: &VirtualDesktopAccessor, desktop: i32) -> Option<HWND> {
    let mut hwnds: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut hwnds as *mut Vec<HWND> as isize));
    }
    for hwnd in hwnds {
        if is_focusable(hwnd) && vd.window_desktop(hwnd.0 as isize) == desktop {
            return Some(hwnd);
        }
    }
    None
}

fn is_focusable(hwnd: HWND) -> bool {
    if hwnd.0.is_null() {
        return false;
    }
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() || !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return false;
        }
    }
    let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    if ex_style & WS_EX_TOOLWINDOW.0 != 0 {
        return false;
    }
    !window_text(hwnd).is_empty()
}

fn window_text(hwnd: HWND) -> String {
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; len as usize + 1];
    let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n as usize])
}
