//! Gapless master + stack tiler.
//!
//! * 1 tiled window on a monitor -> maximized.
//! * 2+ tiled windows -> master column on the left, stack column on the right.
//! * Sizes use `DWMWA_EXTENDED_FRAME_BOUNDS` so Windows' invisible resize
//!   borders don't produce gaps between neighbours.

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;

use log::debug;
use windows::core::{BOOL, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow,
    IsWindowVisible, IsZoomed, SetWindowPos, ShowWindow, GWL_EXSTYLE, GWL_STYLE, SWP_NOACTIVATE,
    SWP_NOOWNERZORDER, SWP_NOZORDER, SW_MAXIMIZE, SW_RESTORE, WS_CAPTION, WS_EX_TOOLWINDOW,
    WS_POPUP,
};

use crate::commands::Direction;
use crate::vda::VirtualDesktopAccessor;

fn hwnd_from(h: isize) -> HWND {
    HWND(h as *mut c_void)
}

pub struct Tiler {
    vd: VirtualDesktopAccessor,
    tile_order: HashMap<(i32, isize), Vec<isize>>,
    owned: HashSet<isize>,
    floating: HashSet<isize>,
    master_ratio: f64,
    min_tile_width: i32,
    min_tile_height: i32,
    tile_interval_ms: u64,
    float_process_names: HashSet<String>,
    float_class_names: HashSet<String>,
}

impl Tiler {
    pub fn new(vd: VirtualDesktopAccessor, config: &crate::config::Config) -> Self {
        Self {
            vd,
            tile_order: HashMap::new(),
            owned: HashSet::new(),
            floating: HashSet::new(),
            master_ratio: config.master_ratio,
            min_tile_width: config.min_tile_width,
            min_tile_height: config.min_tile_height,
            tile_interval_ms: config.tile_interval_ms,
            float_process_names: config
                .float_process_names
                .iter()
                .map(|s| s.to_lowercase())
                .collect(),
            float_class_names: config
                .float_class_names
                .iter()
                .map(|s| s.to_lowercase())
                .collect(),
        }
    }

    pub fn tile_current(&mut self) {
        let start = std::time::Instant::now();
        let desktop = self.vd.current();
        self.tile_desktop(desktop);
        debug!("tile pass took {}ms", start.elapsed().as_millis());
    }

    pub fn interval_ms(&self) -> u64 {
        self.tile_interval_ms
    }

    /// Tiled windows on a desktop, in Z-order (what focus navigation follows).
    pub fn ordered_windows(&self, desktop: i32) -> Vec<isize> {
        self.collect_windows(desktop)
    }

    /// Moves the focused window one slot in the tile order (Win+Shift+H/J/K/L).
    pub fn move_focused(&mut self, direction: Direction) {
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.0.is_null() {
            return;
        }
        let id = hwnd.0 as isize;
        let desktop = self.vd.current();
        let monitor = self.monitor_for_window(hwnd);
        let key = (desktop, monitor);

        if !self.tile_order.contains_key(&key) {
            self.tile_desktop(desktop);
            if !self.tile_order.contains_key(&key) {
                return;
            }
        }

        let order = self.tile_order.get_mut(&key).unwrap();
        let mut idx = 0usize;
        for (i, h) in order.iter().enumerate() {
            if *h == id {
                idx = i + 1;
                break;
            }
        }
        if idx == 0 {
            return;
        }

        let n = order.len();
        let target = if direction.is_prev() {
            if idx == 1 { 1 } else { idx - 1 }
        } else {
            if idx == n { n } else { idx + 1 }
        };

        if target != idx {
            order.swap(idx - 1, target - 1);
        }
        self.tile_current();
    }

    fn monitor_for_window(&self, hwnd: HWND) -> isize {
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
            return 0;
        }
        let cx = rect.left + (rect.right - rect.left) / 2;
        let cy = rect.top + (rect.bottom - rect.top) / 2;
        let monitor =
            unsafe { MonitorFromPoint(POINT { x: cx, y: cy }, MONITOR_DEFAULTTONEAREST) };
        monitor.0 as isize
    }

    /// Toggles the focused window between floating and tiled.
    pub fn toggle_float(&mut self) {
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.0.is_null() {
            return;
        }
        let id = hwnd.0 as isize;
        if self.floating.contains(&id) {
            self.floating.remove(&id);
        } else {
            self.floating.insert(id);
            // A maximized window can't be resized; restore it before floating.
            if self.is_maximized(hwnd) {
                unsafe {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
            }
        }
        self.tile_current();
    }

    /// Snaps the focused window to a half of its monitor and marks it floating.
    pub fn snap(&mut self, direction: Direction) {
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.0.is_null() {
            return;
        }
        let id = hwnd.0 as isize;
        let monitor = self.monitor_for_window(hwnd);
        let Some(work) = monitor_work_area(HMONITOR(monitor as *mut c_void)) else {
            return;
        };

        let mid_x = work.left + (work.right - work.left) / 2;
        let mid_y = work.top + (work.bottom - work.top) / 2;

        self.floating.insert(id);
        if self.is_maximized(hwnd) {
            unsafe {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
        }

        let (x, y, w, h) = match direction {
            Direction::Left => (work.left, work.top, mid_x - work.left, work.bottom - work.top),
            Direction::Right => (mid_x, work.top, work.right - mid_x, work.bottom - work.top),
            Direction::Up => (work.left, work.top, work.right - work.left, mid_y - work.top),
            Direction::Down => (work.left, mid_y, work.right - work.left, work.bottom - mid_y),
        };
        self.place(hwnd, x, y, w, h);
    }

    fn tile_desktop(&mut self, desktop: i32) {
        let windows = self.collect_windows(desktop);
        debug!("tile desktop {desktop}: {} windows", windows.len());
        let by_monitor = self.group_by_monitor(&windows);

        for (monitor, wins) in by_monitor {
            let ordered = self.apply_order(desktop, monitor, wins);

            if ordered.len() == 1 {
                // A single tiled window is truly maximized, like clicking the
                // maximize button, rather than resized to the monitor bounds.
                self.maximize_single(hwnd_from(ordered[0]));
                continue;
            }

            // Two or more windows: maximized ones must join the split.
            for h in &ordered {
                let hwnd = hwnd_from(*h);
                if self.is_maximized(hwnd) {
                    unsafe {
                        let _ = ShowWindow(hwnd, SW_RESTORE);
                    }
                }
            }

            if let Some(work) = monitor_work_area(HMONITOR(monitor as *mut c_void)) {
                self.layout(ordered, work);
            }
        }
    }

    fn collect_windows(&self, desktop: i32) -> Vec<isize> {
        let mut hwnds: Vec<HWND> = Vec::new();
        unsafe {
            let _ = EnumWindows(Some(enum_proc), LPARAM(&mut hwnds as *mut Vec<HWND> as isize));
        }

        let mut out = Vec::new();
        for hwnd in hwnds {
            let id = hwnd.0 as isize;
            if self.is_usable(hwnd)
                && self.vd.window_desktop(id) == desktop
                && !self.is_excluded(hwnd)
                && !self.floating.contains(&id)
                && !self.is_fullscreen(hwnd)
            {
                out.push(id);
            }
        }
        out
    }

    fn group_by_monitor(&self, windows: &[isize]) -> HashMap<isize, Vec<isize>> {
        let mut by_monitor: HashMap<isize, Vec<isize>> = HashMap::new();
        for h in windows {
            let hwnd = hwnd_from(*h);
            let mut rect = RECT::default();
            if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
                continue;
            }
            let cx = rect.left + (rect.right - rect.left) / 2;
            let cy = rect.top + (rect.bottom - rect.top) / 2;
            let monitor =
                unsafe { MonitorFromPoint(POINT { x: cx, y: cy }, MONITOR_DEFAULTTONEAREST) };
            by_monitor.entry(monitor.0 as isize).or_default().push(*h);
        }
        by_monitor
    }

    fn apply_order(&mut self, desktop: i32, monitor: isize, windows: Vec<isize>) -> Vec<isize> {
        let key = (desktop, monitor);
        let old = self.tile_order.get(&key).cloned().unwrap_or_default();

        let mut result: Vec<isize> = Vec::new();
        for h in &old {
            if windows.contains(h) {
                result.push(*h);
            }
        }
        for h in &windows {
            if !result.contains(h) {
                result.push(*h);
            }
        }

        self.tile_order.insert(key, result.clone());
        result
    }

    fn layout(&mut self, windows: Vec<isize>, work: RECT) {
        let count = windows.len();
        if count == 0 {
            return;
        }

        let width = work.right - work.left;
        let height = work.bottom - work.top;

        if count == 1 {
            self.maximize_single(hwnd_from(windows[0]));
            return;
        }

        let master_w = ((width as f64 * self.master_ratio).round() as i32)
            .clamp(self.min_tile_width, width - self.min_tile_width);
        let stack_x = work.left + master_w;
        let stack_w = work.right - stack_x;

        self.place(hwnd_from(windows[0]), work.left, work.top, master_w, height);

        let stack_count = count - 1;
        for i in 0..stack_count {
            let y1 = work.top + ((height as f64 * i as f64 / stack_count as f64).round() as i32);
            let y2 =
                work.top + ((height as f64 * (i + 1) as f64 / stack_count as f64).round() as i32);
            self.place(hwnd_from(windows[i + 1]), stack_x, y1, stack_w, y2 - y1);
        }
    }

    fn place(&mut self, hwnd: HWND, x: i32, y: i32, w: i32, h: i32) {
        if w < self.min_tile_width || h < self.min_tile_height {
            return;
        }
        if self.is_fullscreen(hwnd) {
            return;
        }
        if self.matches_target(hwnd, x, y, w, h) {
            self.owned.insert(hwnd.0 as isize);
            return;
        }

        // Expand the classic rect by the invisible resize border so the
        // visible frame lands exactly on the requested tile.
        let (il, it, ir, ib) = frame_insets(hwnd);
        let mut cx = x - il;
        let mut cy = y - it;
        let mut cw = w + il + ir;
        let mut ch = h + it + ib;
        let flags = SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOOWNERZORDER;
        unsafe {
            let _ = SetWindowPos(hwnd, None, cx, cy, cw, ch, flags);
        }

        // Measure the actual visible frame and correct residual offset (DPI
        // rounding, cloaked-window DWM oddities, changed insets).
        for _ in 0..3 {
            let Some(frame) = visible_frame(hwnd) else {
                break;
            };
            let dx = x - frame.left;
            let dy = y - frame.top;
            let dw = w - (frame.right - frame.left);
            let dh = h - (frame.bottom - frame.top);
            if dx.abs() <= 1 && dy.abs() <= 1 && dw.abs() <= 1 && dh.abs() <= 1 {
                break;
            }

            let mut rect = RECT::default();
            if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
                break;
            }
            cx = rect.left + dx;
            cy = rect.top + dy;
            cw = (rect.right - rect.left) + dw;
            ch = (rect.bottom - rect.top) + dh;
            unsafe {
                let _ = SetWindowPos(hwnd, None, cx, cy, cw, ch, flags);
            }
        }

        self.owned.insert(hwnd.0 as isize);
    }

    fn maximize_single(&mut self, hwnd: HWND) {
        if !self.is_maximized(hwnd) {
            unsafe {
                let _ = ShowWindow(hwnd, SW_MAXIMIZE);
            }
        }
        self.owned.insert(hwnd.0 as isize);
    }

    fn is_usable(&self, hwnd: HWND) -> bool {
        if hwnd.0.is_null() {
            return false;
        }
        unsafe {
            IsWindow(Some(hwnd)).as_bool()
                && IsWindowVisible(hwnd).as_bool()
                && !IsIconic(hwnd).as_bool()
        }
    }

    fn is_maximized(&self, hwnd: HWND) -> bool {
        unsafe { IsZoomed(hwnd).as_bool() }
    }

    fn is_excluded(&self, hwnd: HWND) -> bool {
        let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
        if ex_style & WS_EX_TOOLWINDOW.0 != 0 {
            return true;
        }
        if let Some(class) = class_name(hwnd) {
            if self.float_class_names.contains(&class.to_lowercase()) {
                return true;
            }
        }
        if window_text(hwnd).is_empty() {
            return true;
        }
        if let Some(proc) = process_name(hwnd) {
            if self.float_process_names.contains(&proc) {
                return true;
            }
        }
        false
    }

    fn is_fullscreen(&self, hwnd: HWND) -> bool {
        if self.owned.contains(&(hwnd.0 as isize)) {
            return false;
        }
        let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
        // WS_POPUP without WS_CAPTION = borderless.
        if style & WS_POPUP.0 == 0 {
            return false;
        }
        if style & WS_CAPTION.0 != 0 {
            return false;
        }
        // And it must actually cover its monitor.
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
            return false;
        }
        let cx = rect.left + (rect.right - rect.left) / 2;
        let cy = rect.top + (rect.bottom - rect.top) / 2;
        let monitor = unsafe { MonitorFromPoint(POINT { x: cx, y: cy }, MONITOR_DEFAULTTONEAREST) };
        if let Some(full) = monitor_full_bounds(monitor) {
            return rect.left <= full.left + 2
                && rect.top <= full.top + 2
                && rect.right >= full.right - 2
                && rect.bottom >= full.bottom - 2;
        }
        false
    }

    fn matches_target(&self, hwnd: HWND, x: i32, y: i32, w: i32, h: i32) -> bool {
        let Some(frame) = visible_frame(hwnd) else {
            return false;
        };
        (frame.left - x).abs() <= 1
            && (frame.top - y).abs() <= 1
            && ((frame.right - frame.left) - w).abs() <= 1
            && ((frame.bottom - frame.top) - h).abs() <= 1
    }
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let windows = &mut *(lparam.0 as *mut Vec<HWND>);
    windows.push(hwnd);
    BOOL(1)
}

fn class_name(hwnd: HWND) -> Option<String> {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    if n == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buf[..n as usize]))
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

fn process_name(hwnd: HWND) -> Option<String> {
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    if pid == 0 {
        return None;
    }

    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buf = [0u16; 1024];
    let mut size: u32 = (buf.len() * std::mem::size_of::<u16>()) as u32;
    let ok = unsafe {
        QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut size)
    }
    .is_ok();
    unsafe {
        let _ = CloseHandle(handle);
    }
    if !ok {
        return None;
    }

    let len = (size as usize / 2).min(buf.len());
    let full = String::from_utf16_lossy(&buf[..len]);
    std::path::Path::new(&full)
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
}

fn visible_frame(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    let ok = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut RECT as *mut c_void,
            std::mem::size_of::<RECT>() as u32,
        )
    }
    .is_ok();
    if ok && (rect.right - rect.left) > 0 && (rect.bottom - rect.top) > 0 {
        return Some(rect);
    }

    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok() {
        return Some(rect);
    }
    None
}

fn frame_insets(hwnd: HWND) -> (i32, i32, i32, i32) {
    let Some(frame) = visible_frame(hwnd) else {
        return (0, 0, 0, 0);
    };
    let mut classic = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut classic) }.is_err() {
        return (0, 0, 0, 0);
    }
    if (classic.right - classic.left) <= 0 || (classic.bottom - classic.top) <= 0 {
        return (0, 0, 0, 0);
    }
    (
        frame.left - classic.left,
        frame.top - classic.top,
        classic.right - frame.right,
        classic.bottom - frame.bottom,
    )
}

fn monitor_work_area(monitor: HMONITOR) -> Option<RECT> {
    let mut mi = MONITORINFO::default();
    mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    if unsafe { GetMonitorInfoW(monitor, &mut mi) }.as_bool() {
        Some(mi.rcWork)
    } else {
        None
    }
}

fn monitor_full_bounds(monitor: HMONITOR) -> Option<RECT> {
    let mut mi = MONITORINFO::default();
    mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    if unsafe { GetMonitorInfoW(monitor, &mut mi) }.as_bool() {
        Some(mi.rcMonitor)
    } else {
        None
    }
}
