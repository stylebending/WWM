//! Low-level keyboard hook + message loop (main thread).
//!
//! A low-level `WH_KEYBOARD_LL` hook is used rather than `RegisterHotKey`,
//! which is the only way to grab `Win+1..0` (Explorer already owns those).
//!
//! The hook callback only *records* the matched action; the action runs from
//! the normal message loop. Tiling and focus run on a worker thread so this
//! loop stays instant.

use std::cell::RefCell;
use std::sync::mpsc::Sender;

use log::{debug, info};
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetForegroundWindow, IsZoomed, PeekMessageW,
    PostMessageW, PostQuitMessage, SetWindowsHookExW, ShowWindow, TranslateMessage,
    KBDLLHOOKSTRUCT, MSG, PM_REMOVE, SW_MAXIMIZE, SW_RESTORE, WH_KEYBOARD_LL, WM_CLOSE,
    WM_KEYDOWN, WM_QUIT, WM_SYSKEYDOWN,
};

use crate::commands::Command;
use crate::config::{Action, Keybind, Mods};
use crate::overlay::Overlay;
use crate::vda::VirtualDesktopAccessor;

thread_local! {
    static VDA: RefCell<Option<VirtualDesktopAccessor>> = const { RefCell::new(None) };
    static PENDING: RefCell<Vec<Action>> = const { RefCell::new(Vec::new()) };
    static WORKER_TX: RefCell<Option<Sender<Command>>> = const { RefCell::new(None) };
    static OVERLAY: RefCell<Option<Overlay>> = const { RefCell::new(None) };
    static KEYBINDS: RefCell<Option<Vec<Keybind>>> = const { RefCell::new(None) };
}

pub fn run(
    vd: VirtualDesktopAccessor,
    worker_tx: Sender<Command>,
    keybinds: Vec<Keybind>,
    overlay_ms: u64,
) {
    VDA.with(|c| *c.borrow_mut() = Some(vd));
    WORKER_TX.with(|c| *c.borrow_mut() = Some(worker_tx));
    OVERLAY.with(|c| *c.borrow_mut() = Overlay::new(overlay_ms));
    KEYBINDS.with(|c| *c.borrow_mut() = Some(keybinds));

    let hook = unsafe {
        let module = GetModuleHandleW(None).ok().unwrap_or_default();
        SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(keyboard_proc),
            Some(HINSTANCE(module.0)),
            0,
        )
    }
    .expect("install WH_KEYBOARD_LL hook");

    info!("keyboard hook installed; press Win+Ctrl+Esc to quit");
    message_loop();

    let _ = hook;
    std::process::exit(0);
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let hs = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let down = wparam.0 as u32 == WM_KEYDOWN || wparam.0 as u32 == WM_SYSKEYDOWN;
        if down {
            let mods = Mods {
                win: is_down(VK_LWIN) || is_down(VK_RWIN),
                shift: is_down(VK_SHIFT),
                ctrl: is_down(VK_CONTROL),
                alt: is_down(VK_MENU),
            };
            if let Some(action) = lookup_action(mods, hs.vkCode) {
                debug!("hook matched action={action:?}");
                PENDING.with(|q| q.borrow_mut().push(action));
                return LRESULT(1); // swallow the key
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

fn lookup_action(mods: Mods, vk: u32) -> Option<Action> {
    KEYBINDS.with(|k| {
        k.borrow()
            .as_ref()
            .and_then(|binds| binds.iter().find(|b| b.mods == mods && b.vk == vk))
            .map(|b| b.action.clone())
    })
}

fn is_down(vk: VIRTUAL_KEY) -> bool {
    // GetAsyncKeyState returns i16; the high bit (key currently down) makes it negative.
    unsafe { GetAsyncKeyState(vk.0 as i32) < 0 }
}

fn send_worker(cmd: Command) {
    WORKER_TX.with(|tx| {
        if let Some(tx) = tx.borrow().as_ref() {
            let _ = tx.send(cmd);
        }
    });
}

fn dispatch(action: Action) {
    VDA.with(|c| {
        let vd = c.borrow();
        let vd = match vd.as_ref() {
            Some(vd) => vd,
            None => return,
        };

        match action {
            Action::Switch(n) => {
                // Win+N lands on workspace N+1; create missing desktops first.
                while vd.count() <= n {
                    let created = vd.create_desktop();
                    if created < 0 {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(120));
                }

                let ok = vd.go_to(n);
                info!("switch to desktop {n} -> ok={ok}");
                if ok {
                    send_worker(Command::FocusAfterSwitch);
                }
            }
            Action::Move(n) => {
                let hwnd = unsafe { GetForegroundWindow() };
                if !hwnd.0.is_null() {
                    let ok = vd.move_window(hwnd.0 as isize, n);
                    info!("move {hwnd:?} to desktop {n} -> ok={ok}");
                }
            }
            Action::Focus(dir) => send_worker(Command::Focus(dir)),
            Action::Reorder(dir) => send_worker(Command::Reorder(dir)),
            Action::Float => send_worker(Command::ToggleFloat),
            Action::Snap(dir) => send_worker(Command::Snap(dir)),
            Action::Overlay => {
                let text = format!("WORKSPACE {} of {}", vd.current() + 1, vd.count());
                OVERLAY.with(|o| {
                    if let Some(o) = o.borrow_mut().as_mut() {
                        o.show(&text);
                    }
                });
            }
            Action::Close => {
                let hwnd = unsafe { GetForegroundWindow() };
                if !hwnd.0.is_null() {
                    info!("close {hwnd:?}");
                    unsafe {
                        let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                    }
                }
            }
            Action::Maximize => {
                let hwnd = unsafe { GetForegroundWindow() };
                if !hwnd.0.is_null() {
                    info!("toggle maximize {hwnd:?}");
                    if unsafe { IsZoomed(hwnd) }.as_bool() {
                        unsafe { let _ = ShowWindow(hwnd, SW_RESTORE); }
                    } else {
                        unsafe { let _ = ShowWindow(hwnd, SW_MAXIMIZE); }
                    }
                }
            }
            Action::Quit => {
                info!("quit requested");
                unsafe { PostQuitMessage(0) };
            }
            Action::Run(cmd) => match std::process::Command::new(&cmd).spawn() {
                Ok(_) => info!("launched {cmd}"),
                Err(e) => info!("failed to launch {cmd}: {e}"),
            },
        }
    });
}

fn message_loop() {
    let mut msg = MSG::default();
    loop {
        // Drive the low-level keyboard hook and process posted messages.
        let has_msg = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool();
        if has_msg {
            if msg.message == WM_QUIT {
                break;
            }
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        // Run hotkey actions recorded by the hook callback.
        let pending: Vec<Action> = PENDING.with(|q| q.borrow_mut().drain(..).collect());
        for action in pending {
            dispatch(action);
        }

        // Auto-hide the overlay after its timeout.
        OVERLAY.with(|o| {
            if let Some(o) = o.borrow_mut().as_mut() {
                o.tick();
            }
        });
    }
}
