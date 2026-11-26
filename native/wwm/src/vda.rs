//! Virtual-desktop access, inlined via the `winvd` crate (no DLL).
//!
//! `winvd` calls the undocumented Windows 11 virtual-desktop COM interfaces
//! directly and compiles into this executable.

use std::ffi::c_void;
use std::path::PathBuf;

use windows058::Win32::Foundation::HWND as HWND058;

#[derive(Clone)]
pub struct VirtualDesktopAccessor;

impl VirtualDesktopAccessor {
    pub fn load() -> Result<Self, String> {
        // Sanity-check that the virtual-desktop COM service is reachable.
        let _ = winvd::get_desktop_count()
            .map_err(|e| format!("virtual desktop COM unavailable: {e:?}"))?;
        Ok(Self)
    }

    pub fn count(&self) -> i32 {
        winvd::get_desktop_count().map(|n| n as i32).unwrap_or(1)
    }

    pub fn current(&self) -> i32 {
        winvd::get_current_desktop()
            .and_then(|d| d.get_index().map(|i| i as i32))
            .unwrap_or(0)
    }

    pub fn go_to(&self, desktop: i32) -> bool {
        winvd::switch_desktop(desktop as u32).is_ok()
    }

    pub fn move_window(&self, hwnd: isize, desktop: i32) -> bool {
        let h = hwnd058(hwnd);
        winvd::move_window_to_desktop(desktop as u32, &h).is_ok()
    }

    pub fn window_desktop(&self, hwnd: isize) -> i32 {
        let h = hwnd058(hwnd);
        winvd::get_desktop_by_window(h)
            .and_then(|d| d.get_index().map(|i| i as i32))
            .unwrap_or(-1)
    }

    #[allow(dead_code)]
    pub fn is_on_current(&self, hwnd: isize) -> bool {
        let h = hwnd058(hwnd);
        winvd::is_window_on_current_desktop(h).unwrap_or(false)
    }

    pub fn create_desktop(&self) -> i32 {
        winvd::create_desktop()
            .and_then(|d| d.get_index().map(|i| i as i32))
            .unwrap_or(-1)
    }

    pub fn remove_desktop(&self, desktop: i32, fallback: i32) -> bool {
        winvd::remove_desktop(desktop as u32, fallback as u32).is_ok()
    }
}

fn hwnd058(hwnd: isize) -> HWND058 {
    HWND058(hwnd as *mut c_void)
}

/// Walks up from the exe directory looking for a file beside the checkout.
pub(crate) fn find_file_upward(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent()?.to_path_buf();
    for _ in 0..6 {
        let candidate = dir.join(name);
        if candidate.exists() {
            return Some(candidate);
        }
        if !dir.pop() {
            break;
        }
    }
    None
}
