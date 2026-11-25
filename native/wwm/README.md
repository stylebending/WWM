# WWM (native Rust)

A native tiling window manager for Windows 11 virtual desktops. Single
self-contained executable — virtual-desktop and focus COM are compiled in via
the `winvd` crate and hand-written `IApplicationView` bindings.

## Current state

- Virtual desktops: switch / move / create via `winvd` (no DLL).
- `WH_KEYBOARD_LL` keyboard hook (required to grab `Win+1..0`).
- Gapless master + stack tiling; single window -> maximized; work-area aware.
- Directional focus, reorder, snap, float toggle, workspace overlay.
- Configurable keybindings and app launchers (`wwm.toml`).
- Focus/flash handling after a switch (inline `IApplicationView::SetFocus`,
  with a `SetForegroundWindow` fallback).
- Tiling and focus run on a worker thread; the hotkey thread only performs the
  desktop switch, so switching stays instant.

## Build

```bat
cargo build --release
```

The executable is `target\release\wwm.exe` (~450 KB, no runtime).

## Run

```bat
target\release\wwm.exe
```

Optional `wwm.toml` beside the executable. Set `WWM_LOG=debug` for a verbose
`wwm.log` beside the executable.
