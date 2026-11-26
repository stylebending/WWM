# Windows Workspace Manager (WWM)

A lightweight tiling window manager for **Windows 11 native virtual
desktops**, written in Rust. It drives the virtual desktops Windows already has
and tiles windows into a master + stack layout.

## Requirements

- Windows 11 24H2 build 26100.2605 or newer (25H2 recommended), x64.

## Build

```bat
cd native\wwm
cargo build --release
```

The executable is `native\wwm\target\release\wwm.exe` (~450 KB).

## Install

Copy `wwm.exe` wherever you like, optionally with a `wwm.toml` beside it.
That's it, no other files are needed.

Run `wwm.exe` (double-click or from a terminal).

## Run at startup

1. Press `Win+R`, type `shell:startup`, and press Enter (this opens the
   Startup folder).
2. Drop a shortcut to `wwm.exe` into that folder.

It will start automatically when you log in. Keep `wwm.toml` next to the
executable (or anywhere above it) so the config is found. If you later need
WWM to manage elevated windows, use Task Scheduler with "Run with highest
privileges" instead of the Startup folder.

## Keybindings (defaults)

| Shortcut | Action |
|---|---|
| `Win+1..9`, `Win+0` | Switch to workspace 1..10 (creates it if missing) |
| `Win+Shift+1..9`, `Win+Shift+0` | Move the focused window to a workspace |
| `Win+H` / `Win+J` / `Win+K` / `Win+L` | Focus previous / down / up / next tile |
| `Win+Shift+H/J/K/L` | Reorder the focused tile |
| `Win+Ctrl+H/J/K/L` | Snap the focused window left/right/top/bottom |
| `Win+Shift+Space` | Toggle the focused window floating/tiled |
| `Win+Space` | Show the workspace overlay |
| `Win+Q` | Close the focused window |
| `Win+F` | Toggle maximize/restore |
| `Win+B` / `Win+E` / `Win+T` / `Win+N` | Launch Brave / Explorer / Terminal / Notepad |
| `Win+Ctrl+Esc` | Quit |

All keybindings are configurable (see below).

## Configuration

`wwm.toml` beside the executable (or anywhere within 6 levels above it). All
sections are optional; missing entries use the defaults baked into the binary.

```toml
[tiling]
master_ratio = 0.5          # master column width fraction
tile_interval_ms = 250      # tiler poll interval
min_tile_width = 160
min_tile_height = 120

[float]
process_names = ["ApplicationFrameHost.exe", "..."]

[overlay]
duration_ms = 1100

[keybinds]
"win+1" = "switch 1"
"win+h" = "focus left"
"win+b" = "run brave.exe"
```

Keybind actions: `switch N`, `move N`, `focus <dir>`, `reorder <dir>`,
`snap <dir>`, `float`, `overlay`, `close`, `maximize`, `quit`, and
`run <command>` (any PATH executable or full path).

## Tiling behaviour

- **One window** on a monitor is maximized.
- **Two or more** split into a master column (left) plus a stack (right),
  using the monitor work area (a visible taskbar is never overlapped).
- Genuine borderless-fullscreen apps (`WS_POPUP` without `WS_CAPTION`) and
  floating windows are left alone.
- Each monitor gets an independent layout.
- Tiling and focus run on a worker thread; the hotkey thread only performs the
  desktop switch, so switching stays instant.

## Notes

- Virtual-desktop access uses the [`winvd`](https://crates.io/crates/winvd)
  crate (MIT), compiled into the executable.
- Focus uses the undocumented `IApplicationView::SetFocus` COM interface
  directly, with a `SetForegroundWindow` fallback.
- If an application runs elevated, run WWM at the same integrity level.
