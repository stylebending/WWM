//! Master + stack tiler.
//!
//! * 1 tiled window on a monitor -> maximized (or inset when `gap > 0`).
//! * 2+ tiled windows -> master column on the left, stack column on the right.
//! * Configurable outer/inner gaps.
//! * Sizes use `DWMWA_EXTENDED_FRAME_BOUNDS` so Windows' invisible resize
//!   borders don't produce unintended gaps between neighbours.

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::sync::{Arc, Mutex};

use log::{debug, info};
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
    desktop_lock: Arc<Mutex<()>>,
    tile_order: HashMap<(i32, isize), Vec<isize>>,
    owned: HashSet<isize>,
    floating: HashSet<isize>,
    tiled: HashSet<isize>,
    master_ratio: f64,
    min_tile_width: i32,
    min_tile_height: i32,
    gap: i32,
    tile_interval_ms: u64,
    default_float: bool,
    float_process_names: HashSet<String>,
    float_class_names: HashSet<String>,
}

impl Tiler {
    pub fn new(
        vd: VirtualDesktopAccessor,
        config: &crate::config::Config,
        desktop_lock: Arc<Mutex<()>>,
    ) -> Self {
        Self {
            vd,
            desktop_lock,
            tile_order: HashMap::new(),
            owned: HashSet::new(),
            floating: HashSet::new(),
            tiled: HashSet::new(),
            master_ratio: config.master_ratio,
            min_tile_width: config.min_tile_width,
            min_tile_height: config.min_tile_height,
            gap: config.gap,
            tile_interval_ms: config.tile_interval_ms,
            default_float: config.default_float,
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

    /// Tiles every existing virtual desktop. Used once at startup so windows
    /// on already-existing desktops don't have to wait until they're visited.
    pub fn tile_all(&mut self) {
        let count = self.vd.count();
        for desktop in 0..count {
            self.tile_desktop(desktop);
        }
    }

    pub fn interval_ms(&self) -> u64 {
        self.tile_interval_ms
    }

    fn is_floating(&self, id: isize) -> bool {
        if self.tiled.contains(&id) {
            false
        } else if self.floating.contains(&id) {
            true
        } else {
            self.default_float
        }
    }

    /// Tiled windows on a desktop, in Z-order (what focus navigation follows).
    pub fn ordered_windows(&self, desktop: i32) -> Vec<isize> {
        self.collect_windows(desktop)
    }

    /// Deletes empty non-current virtual desktops. The current desktop is left
    /// alone so the user always has a place to launch their next window.
    pub fn prune_empty_desktops(&mut self) {
        let _guard = self.desktop_lock.lock().unwrap();
        loop {
            let current = self.vd.current();
            let count = self.vd.count();
            if count <= 1 {
                return;
            }

            let mut deleted = false;
            for desktop in 0..count {
                if desktop == current {
                    continue;
                }
                if !self.desktop_has_windows(desktop)
                    && self.vd.remove_desktop(desktop, current)
                {
                    info!("auto-deleted empty desktop {desktop}");
                    deleted = true;
                    break;
                }
            }

            if !deleted {
                return;
            }
        }
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
        if self.is_floating(id) {
            self.floating.remove(&id);
            self.tiled.insert(id);
        } else {
            self.tiled.remove(&id);
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

        self.tiled.remove(&id);
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
        self.prune_stale_handles();
        let windows = self.collect_windows(desktop);
        debug!("tile desktop {desktop}: {} windows", windows.len());
        let by_monitor = self.group_by_monitor(&windows);

        for (monitor, wins) in by_monitor {
            let ordered = self.apply_order(desktop, monitor, wins);

            if ordered.len() == 1 {
                let hwnd = hwnd_from(ordered[0]);
                if self.gap > 0 {
                    if let Some(work) = monitor_work_area(HMONITOR(monitor as *mut c_void)) {
                        self.layout_single(hwnd, work);
                    }
                } else {
                    // A single tiled window is truly maximized, like clicking the
                    // maximize button, rather than resized to the monitor bounds.
                    self.maximize_single(hwnd);
                }
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
        let mut out = Vec::new();
        for hwnd in self.all_windows() {
            let id = hwnd.0 as isize;
            if self.is_usable(hwnd)
                && self.vd.window_desktop(id) == desktop
                && !self.is_excluded(hwnd)
                && !self.is_floating(id)
                && !self.is_fullscreen(hwnd)
            {
                out.push(id);
            }
        }
        out
    }

    fn desktop_has_windows(&self, desktop: i32) -> bool {
        self.all_windows().into_iter().any(|hwnd| {
            let id = hwnd.0 as isize;
            self.is_real_window(hwnd) && self.vd.window_desktop(id) == desktop
        })
    }

    /// A window that counts as a real app window (minimized windows count
    /// too, unlike the tiler's usable-window check). System classes and tool
    /// windows are ignored, but empty titles are not, so apps without a
    /// visible title still keep their desktop alive.
    fn is_real_window(&self, hwnd: HWND) -> bool {
        if hwnd.0.is_null() {
            return false;
        }
        unsafe {
            if !IsWindow(Some(hwnd)).as_bool() || !IsWindowVisible(hwnd).as_bool() {
                return false;
            }
        }
        let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
        if ex_style & WS_EX_TOOLWINDOW.0 != 0 {
            return false;
        }
        if let Some(class) = class_name(hwnd) {
            if self.float_class_names.contains(&class.to_lowercase()) {
                return false;
            }
        }
        true
    }

    fn all_windows(&self) -> Vec<HWND> {
        let mut hwnds: Vec<HWND> = Vec::new();
        unsafe {
            let _ = EnumWindows(Some(enum_proc), LPARAM(&mut hwnds as *mut Vec<HWND> as isize));
        }
        hwnds
    }

    /// Drop per-window state for handles that no longer exist, so a reused
    /// HWND doesn't inherit an old float/tile override.
    fn prune_stale_handles(&mut self) {
        let live: HashSet<isize> = self
            .all_windows()
            .into_iter()
            .map(|h| h.0 as isize)
            .collect();
        self.floating.retain(|h| live.contains(h));
        self.tiled.retain(|h| live.contains(h));
        self.owned.retain(|h| live.contains(h));
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

        if count == 1 {
            let hwnd = hwnd_from(windows[0]);
            if self.gap > 0 {
                self.layout_single(hwnd, work);
            } else {
                self.maximize_single(hwnd);
            }
            return;
        }

        let gap = self.gap;
        let area_left = work.left + gap;
        let area_top = work.top + gap;
        let area_right = work.right - gap;
        let area_bottom = work.bottom - gap;
        let area_w = area_right - area_left;
        let area_h = area_bottom - area_top;

        let max_master = area_w - gap - self.min_tile_width;
        let master_w = if max_master < self.min_tile_width {
            // Not enough room for a gap and two minimum-size tiles.
            area_w / 2
        } else {
            (((area_w - gap) as f64 * self.master_ratio).round() as i32)
                .clamp(self.min_tile_width, max_master)
        };
        let stack_x = area_left + master_w + gap;
        let stack_w = area_right - stack_x;

        self.place(hwnd_from(windows[0]), area_left, area_top, master_w, area_h);

        let stack_count = count - 1;
        let total_gap = gap * (stack_count as i32 - 1);
        let available = area_h - total_gap;
        for i in 0..stack_count {
            let y1 = area_top
                + ((available as f64 * i as f64 / stack_count as f64).round() as i32)
                + gap * i as i32;
            let y2 = area_top
                + ((available as f64 * (i + 1) as f64 / stack_count as f64).round() as i32)
                + gap * i as i32;
            self.place(hwnd_from(windows[i + 1]), stack_x, y1, stack_w, y2 - y1);
        }
    }

    fn layout_single(&mut self, hwnd: HWND, work: RECT) {
        if self.is_maximized(hwnd) {
            unsafe {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
        }
        let gap = self.gap;
        self.place(
            hwnd,
            work.left + gap,
            work.top + gap,
            (work.right - work.left) - gap * 2,
            (work.bottom - work.top) - gap * 2,
        );
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
