# Windows Workspace Manager (WWM) — AutoHotkey v2

A lightweight Hyprland/i3-inspired Windows 11 workspace and window-management setup.

## Included

- Virtual desktop switching and moving windows using Ciantic's `VirtualDesktopAccessor.dll`.
- Per-desktop focus history for `Win+Alt+H` cycling.
- Optional native `FocusBridge.dll` source implementing the foreground-focus COM workaround described in Ciantic/VirtualDesktopAccessor issue #101.
- Gapless automatic tiling: fullscreen for one window, master + stack for more, grouped per monitor.
- Floating-window toggle and explicit snap zones.
- Order-based focus navigation and keyboard-driven tile reordering.
- Small numbered workspace overlay (no workspace names).
- Brave, Explorer, Windows Terminal and Notepad launchers.

## Requirements

- Windows 11 24H2 or newer (25H2 recommended).
- AutoHotkey v2 x64.
- Ciantic's current x64 `VirtualDesktopAccessor.dll`.
- Optional: Visual Studio C++ Build Tools to compile `FocusBridge.dll`.

Ciantic's README says the current DLL requires Windows 11 24H2 build 26100.2605 or newer and reports testing on 25H2 build 26200.8117. Download the DLL from the repository's Releases page:
https://github.com/Ciantic/VirtualDesktopAccessor/releases

Put `VirtualDesktopAccessor.dll` beside `WWM.ahk`.

## Build the native focus bridge

The COM bridge uses undocumented Windows shell interfaces (`IApplicationView` and `IApplicationViewCollection`). The relevant interface ABI can change with Windows updates.

1. Open **x64 Native Tools Command Prompt for Visual Studio**.
2. `cd` into `WWM\native\FocusBridge`.
3. Run `build-x64.bat`.
4. The script copies `FocusBridge.dll` into the WWM root.

The bridge implements the focus sequence documented in issue #101:
`PrepareWindowFocus(hwnd)` -> `GoToDesktopNumber(target)` -> `CommitWindowFocus()`.

If the bridge is not present, the AHK script falls back to `WinActivate`; this fallback may still produce the taskbar-flashing behaviour. Do not treat the fallback as equivalent to the COM bridge.

## Install

1. Extract the whole folder to a stable path, e.g. `C:\Tools\WWM`.
2. Download the current x64 `VirtualDesktopAccessor.dll` from Ciantic's Releases page and place it in the root folder.
3. (Recommended) Build `FocusBridge.dll` using the steps above.
4. Double-click `WWM.ahk`.
5. To start it at login, press `Win+R`, run `shell:startup`, and put a shortcut to `WWM.ahk` there.

## Keybindings

| Shortcut | Action |
|---|---|
| `Win+1` ... `Win+9`, `Win+0` | Switch to workspace 1 ... 10 |
| `Win+Shift+1` ... `Win+Shift+0` | Move active window to workspace |
| `Win+Ctrl+Left/Right` | Previous/next workspace |
| `Win+Ctrl+N` | Create and switch to a workspace |
| `Win+H/J/K` | Focus previous/down/up tile |
| `Win+Alt+H` | Cycle focus back through history |
| `Win+Shift+H/J/K/L` | Reorder focused tiled window |
| `Win+Ctrl+H/J/K/L` | Snap active window left/bottom/top/right |
| `Win+Shift+Space` | Toggle floating/tiled for active window |
| `Win+Space` / `Win+Tab` | Show workspace overlay |
| `Win+B` | Brave |
| `Win+E` | Explorer |
| `Win+T` | Windows Terminal |
| `Win+N` | Notepad |
| `Win+Q` | Close active window |
| `Win+F` | Toggle maximize |
| `Win+Alt+L` | Lock the session |
| `Win+Ctrl+R` | Reload script |
| `Win+Ctrl+Esc` | Exit manager |

### `Win+L` is reserved by Windows

`Win+L` cannot be reliably rebound from userspace. Winlogon handles the lock
shortcut on a path that can win the race against the keyboard hook, so the
`$#l::` focus-right hotkey will normally lock the session instead of moving
focus. WWM therefore treats `Win+L` as reserved: use `Win+H/J/K` for
left/down/up focus, and `Win+Alt+L` to lock explicitly.

A best-effort opt-in exists for machines where the hook is not enough: set
`WWM_DisableWinLock := true` in `config/settings.ahk`. WWM then writes a
per-user policy that removes the OS `Win+L` binding and restores it on exit.
This can help on some Windows builds, but it is not guaranteed — treat `Win+L`
as reserved either way.

## Tiling behaviour

- **One window fills the monitor** (Hyprland-style fullscreen); **two or more**
split into a master column plus a stack, 50/50 by default. Tiles use the full
monitor bounds, not the work area, so there is never a reserved taskbar gap.
- Completely gapless. Windows' invisible resize border is measured once per
window (`DWMWA_EXTENDED_FRAME_BOUNDS`) and the classic window rect is expanded
by that border, so the visible frame lands exactly on the tile.
- A genuine borderless-fullscreen app (`WS_POPUP` without `WS_CAPTION`,
covering its monitor) is left alone, as is any maximized window. Being
monitor-sized is NOT fullscreen; a single-window tile fills the monitor by
design.
- A single maximized window is left alone (treated as fullscreen) rather than
immediately restored. When a second window arrives on the same monitor,
maximized windows are restored so the monitor can split.
- Each monitor gets an independent layout.
- The tiler runs on a slow timer, and once after a workspace switch.
- `Win+Shift+Space` toggles the active window between floating and tiled.
- `Win+Ctrl+H/J/K/L` applies a manual half-screen snap and marks it floating.
- Focus navigation follows the tile order, so left and right are exact inverses
and wrap around the monitor.

### Workspace switching and the taskbar flash

`VirtualDesktopAccessor`'s `GoToDesktopNumber` does **not** move the foreground
window. After switching, `GetForegroundWindow()` still returns a window on the
desktop you left (`IsWindowOnCurrentVirtualDesktop` = 0). Windows then shows
nothing focused, the taskbar looks focused, and that stale window blinks for
attention.

For hotkey switches Windows Workspace Manager switches desktops first, then
immediately focuses the incoming desktop's window with the COM focus bridge (which cannot
prepare a window while it is still on another desktop). It retries briefly
while the switch animation finishes and falls back to `WinActivate` with an
immediate flash clear if the bridge is unavailable. A poll fallback covers
shell-initiated switches (e.g. Win+Tab).

## Customization

Edit `config/settings.ahk`:
- `WWM_MasterRatio`: master width fraction (default 0.5).
- `WWM_TileIntervalMs`: window-change polling interval.
- `A_BrowserPath`: set to `brave.exe` or a full path if Brave is not on PATH.

There is intentionally no gap setting; tiling is always gapless.

## Notes and limitations

- The focus bridge relies on undocumented Windows shell COM interfaces. It is source code, not a prebuilt binary; build it locally and verify on your exact Windows build.
- AutoHotkey window enumeration and the shell's cloaking rules can differ for UWP, elevated, and protected windows.
- If an application runs elevated, run Windows Workspace Manager at the same integrity level for window manipulation to work consistently.
- The first pass is a practical floating-window tiler, not a full compositor: no animations; borderless-fullscreen windows are left untouched, and some special windows are excluded.
- Desktop numbering is zero-based in the DLL and one-based in the hotkeys/UI.
