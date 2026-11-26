#![windows_subsystem = "windows"]

//! Windows Workspace Manager — native Rust.
//!
//! Accesses Windows virtual desktops via the `winvd` crate (compiled in),
//! installs a low-level keyboard hook, wires the desktop hotkeys, and runs the
//! master/stack tiler plus focus management on a worker thread so the hotkey
//! path stays instant.

mod commands;
mod config;
mod focus;
mod hook;
mod logging;
mod overlay;
mod tiling;
mod vda;

use std::ffi::c_void;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use log::info;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

use commands::{Command, Direction};
use focus::FocusManager;
use vda::VirtualDesktopAccessor;

fn main() {
    logging::init().expect("init logger");

    // Match DWM's coordinate space (per-monitor DPI aware).
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwareness(
            windows::Win32::UI::HiDpi::PROCESS_PER_MONITOR_DPI_AWARE,
        );
    }

    // VirtualDesktopAccessor uses COM internally; initialize this thread too.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }

    let vd = Arc::new(VirtualDesktopAccessor::load().expect("virtual desktop COM unavailable"));

    let config = config::Config::load();
    info!(
        "WWM native: {} virtual desktops, current {}",
        vd.count(),
        vd.current()
    );

    let keybinds = config.keybinds.clone();
    let overlay_ms = config.overlay_ms;

    // Serializes desktop create/switch/move/remove between the hotkey thread
    // and the tiler worker so indices can't shift mid-operation.
    let desktop_lock = Arc::new(Mutex::new(()));

    // The worker owns tiling + focus/flash-clearing. The main thread only runs
    // the keyboard hook and the actual desktop switch, so switching is instant.
    let (worker_tx, worker_rx) = mpsc::channel::<Command>();
    let worker_vd = vd.clone();
    let worker_lock = desktop_lock.clone();
    std::thread::spawn(move || worker(worker_vd, worker_rx, config, worker_lock));

    hook::run((*vd).clone(), worker_tx, keybinds, overlay_ms, desktop_lock);
}

fn worker(
    vd: Arc<VirtualDesktopAccessor>,
    rx: mpsc::Receiver<Command>,
    config: config::Config,
    desktop_lock: Arc<Mutex<()>>,
) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }

    let mut tiler = tiling::Tiler::new((*vd).clone(), &config, desktop_lock);
    tiler.tile_all();
    let focus = FocusManager::new();

    let tile_interval = Duration::from_millis(tiler.interval_ms());
    let focus_timeout = Duration::from_millis(800);
    let mut last_tile = Instant::now();
    let mut focus_until: Option<Instant> = None;

    loop {
        let now = Instant::now();

        if now.duration_since(last_tile) >= tile_interval {
            tiler.tile_current();
            tiler.prune_empty_desktops();
            last_tile = now;
        }

        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                Command::FocusAfterSwitch => {
                    // Tile the new desktop immediately instead of waiting for
                    // the next tile poll.
                    tiler.tile_current();
                    last_tile = Instant::now();
                    focus.clear_all_flashes();
                    focus_until = Some(Instant::now() + focus_timeout);
                }
                Command::Focus(dir) => {
                    focus_directional(&tiler, &focus, &vd, dir);
                }
                Command::Reorder(dir) => {
                    tiler.move_focused(dir);
                }
                Command::ToggleFloat => {
                    tiler.toggle_float();
                }
                Command::Snap(dir) => {
                    tiler.snap(dir);
                }
            }
        }

        if let Some(until) = focus_until {
            if now > until {
                focus_until = None;
            } else {
                let done = focus.focus_current_desktop(&vd);
                focus.clear_all_flashes();
                if done {
                    focus_until = None;
                }
            }
        }

        std::thread::sleep(Duration::from_millis(40));
    }
}

fn focus_directional(
    tiler: &tiling::Tiler,
    focus: &FocusManager,
    vd: &VirtualDesktopAccessor,
    dir: Direction,
) {
    let desktop = vd.current();
    let active = unsafe { GetForegroundWindow() };
    if active.0.is_null() {
        return;
    }

    let order = tiler.ordered_windows(desktop);
    if order.len() < 2 {
        return;
    }

    let mut idx = 0usize;
    for (i, h) in order.iter().enumerate() {
        if *h == active.0 as isize {
            idx = i + 1;
            break;
        }
    }
    if idx == 0 {
        return;
    }

    let n = order.len();
    let target = if dir.is_prev() {
        if idx == 1 { n } else { idx - 1 }
    } else {
        if idx == n { 1 } else { idx + 1 }
    };

    let hwnd = HWND(order[target - 1] as *mut c_void);
    focus.focus_window(hwnd);
}
