; Gapless Master + Stack tiler.
;
; Layout rules:
;   * 1 tiled window on a monitor -> full monitor work area.
;   * 2+ tiled windows -> master column on the left, stack column on the right.
;   * All edges are derived from shared boundaries, so tiles touch exactly and
;     no 1px seams appear between neighbours.
;
; Sizing uses DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS) instead of the
; classic window rect. Windows 10/11 adds an invisible resize border to normal
; frames; sizing by the classic rect leaves real on-screen gaps. This is the
; usual cause of "gaps I don't want".

global WWM_FloatingWindows := Map()
global WWM_TileOrder := Map()
global WWM_TilingBusy := false
global WWM_TilingSuspended := false
global WWM_LastTiledDesktop := -1
global WWM_ShellHookMsg := 0

; Windows this tiler has positioned itself. A tiled window that fills its
; monitor must NOT be mistaken for a user's fullscreen app, otherwise it gets
; skipped on every later pass and silently drops out of the layout.
global WWM_TilerOwned := Map()

Tiling_Init() {
    global WWM_AutoTileEnabled, WWM_TileIntervalMs
    if WWM_AutoTileEnabled
        SetTimer(Tiling_TileCurrent, WWM_TileIntervalMs)
    Tiling_InitShellHook()
}

; Asks the shell to notify us the moment a top-level window is created or
; destroyed. This makes newly launched programs get tiled immediately instead
; of waiting for the next slow poll tick.
Tiling_InitShellHook() {
    global WWM_ShellHookMsg
    try {
        WWM_ShellHookMsg := DllCall("RegisterWindowMessage", "Str", "SHELLHOOK", "UInt")
        if !WWM_ShellHookMsg
            return
        DllCall("RegisterShellHookWindow", "Ptr", A_ScriptHwnd)
        OnMessage(WWM_ShellHookMsg, Tiling_ShellHook)
    }
}

Tiling_ShellHook(wParam, lParam, msg, hwnd) {
    ; HSHELL_WINDOWCREATED = 1, HSHELL_WINDOWDESTROYED = 2. A freshly created
    ; window is often not visible or titled yet, so retile a few times over the
    ; next half second rather than exactly once.
    ;
    ; This callback runs for every top-level window event, so it must never
    ; throw: one bad hwnd would otherwise pop an error dialog on every event.
    try {
        if wParam = 1 {
            Tiling_ScheduleRetile()
        } else if wParam = 2 {
            ; Drop a closed window from tile order / floating state immediately
            ; so stale hwnds do not linger in the layout maps.
            Tiling_ForgetWindow(lParam)
            Tiling_ScheduleRetile()
        }
    }
}

; Re-tiles the current desktop a few times, spaced out to catch windows that
; only become visible/titled (and therefore eligible) a moment after creation.
Tiling_ScheduleRetile(force := false) {
    Tiling_RetileArmCurrent(force, [40, 180, 500])
}

Tiling_RetileArmCurrent(force, delays) {
    if !delays.Length
        return
    SetTimer(Tiling_RetileFireCurrent.Bind(force, delays), -delays.RemoveAt(1))
}

Tiling_RetileFireCurrent(force, delays) {
    try Tiling_TileCurrent(force)
    Tiling_RetileArmCurrent(force, delays)
}

; Re-tiles a specific desktop a couple of times after a window was moved to it.
Tiling_ScheduleDesktopRetile(desktop) {
    Tiling_RetileArmDesktop(desktop, [200, 600])
}

Tiling_RetileArmDesktop(desktop, delays) {
    if !delays.Length
        return
    SetTimer(Tiling_RetileFireDesktop.Bind(desktop, delays), -delays.RemoveAt(1))
}

Tiling_RetileFireDesktop(desktop, delays) {
    try Tiling_TileDesktop(desktop, true)
    Tiling_RetileArmDesktop(desktop, delays)
}

Tiling_IsWindowOnCurrentDesktop(hwnd) {
    try return VD_Call("IsWindowOnCurrentVirtualDesktop", "Ptr", hwnd, "Int") = 1
    catch
        return false
}

Tiling_IsExcluded(hwnd) {
    global WWM_FloatProcessNames, WWM_FloatClassNames, WWM_OverlayHwnd
    if !hwnd
        return true
    if hwnd = A_ScriptHwnd || (WWM_OverlayHwnd && hwnd = WWM_OverlayHwnd)
        return true
    try {
        if WWM_FloatClassNames.Has(WinGetClass("ahk_id " hwnd))
            return true
        if WinGetExStyle("ahk_id " hwnd) & 0x80 ; WS_EX_TOOLWINDOW
            return true
        if WWM_FloatProcessNames.Has(WinGetProcessName("ahk_id " hwnd))
            return true
        if WinGetTitle("ahk_id " hwnd) = ""
            return true
    } catch {
        return true
    }
    return false
}

; Windows managed by this script that are currently tiled, in tile order.
; Used by both the layout pass and the focus navigator so they always agree.
Tiling_GetOrderedWindows(monitor := 0) {
    global WWM_FloatingWindows
    list := []
    for hwnd in WinGetList() {
        if !VD_IsUsableWindow(hwnd) || !Tiling_IsWindowOnCurrentDesktop(hwnd)
            continue
        if Tiling_IsExcluded(hwnd) || WWM_FloatingWindows.Has(hwnd)
            continue
        try {
            ; Maximized windows are tiled (they are restored first); only true
            ; borderless fullscreen is left alone.
            if Tiling_IsFullscreen(hwnd)
                continue
            WinGetPos(&x, &y, &w, &h, "ahk_id " hwnd)
            if monitor && Tiling_MonitorForRect(x, y, w, h) != monitor
                continue
        } catch {
            continue
        }
        list.Push(hwnd)
    }
    return list
}

Tiling_TileCurrent(force := false) {
    Tiling_TileDesktop(VD_Current(), force)
}

; Tiles a specific virtual desktop. WWM_GetAllWindows is used instead of
; WinGetList because the desktop may not be the current one (e.g. after moving
; a window to another workspace).
Tiling_TileDesktop(desktop, force := false) {
    global WWM_TilingBusy, WWM_AutoTileEnabled, WWM_FloatingWindows, WWM_TilingSuspended
    global WWM_LastTiledDesktop
    if WWM_TilingSuspended || WWM_TilingBusy
        return
    if !WWM_AutoTileEnabled && !force
        return
    WWM_TilingBusy := true
    try {
        byMonitor := Map()
        for hwnd in WWM_GetAllWindows() {
            if !VD_IsUsableWindow(hwnd)
                continue
            if VD_WindowDesktop(hwnd) != desktop
                continue
            if Tiling_IsExcluded(hwnd) || WWM_FloatingWindows.Has(hwnd)
                continue
            try {
                ; Only a genuine borderless fullscreen window is left untouched.
                if Tiling_IsFullscreen(hwnd)
                    continue
                WinGetPos(&x, &y, &w, &h, "ahk_id " hwnd)
                monitor := Tiling_MonitorForRect(x, y, w, h)
                if !byMonitor.Has(monitor)
                    byMonitor[monitor] := []
                byMonitor[monitor].Push(hwnd)
            }
        }

        for monitor, windows in byMonitor {
            ordered := Tiling_ApplyOrder(desktop, monitor, windows)
            if ordered.Length = 1 {
                ; A single tiled window is truly maximized (the same state as
                ; clicking the maximize button), not merely resized to the
                ; monitor bounds. Maximizing gives the correct taskbar-aware
                ; work area and the proper restore/maximize button state.
                Tiling_MaximizeSingle(ordered[1])
                continue
            }
            ; With two or more windows on the monitor, maximized windows must
            ; participate in the split, so restore them before laying out the
            ; master/stack columns.
            for hwnd in ordered {
                if Tiling_IsMaximized(hwnd)
                    WinRestore("ahk_id " hwnd)
            }
            ; Tile to the monitor work area so a visible taskbar is never
            ; overlapped. (With auto-hide, the work area is the full monitor.)
            MonitorGetWorkArea(monitor, &left, &top, &right, &bottom)
            Tiling_LayoutMasterStack(ordered, left, top, right, bottom)
        }
        WWM_LastTiledDesktop := desktop
    } finally {
        WWM_TilingBusy := false
    }
}

Tiling_IsFullscreen(hwnd) {
    ; Only a genuine borderless-fullscreen app counts: games and video players
    ; use a popup window with no caption. This must NOT be "does the window
    ; cover the monitor" -- a normal single-window tile fills the monitor by
    ; design, and treating that as fullscreen made the tiler skip every window,
    ; which is why a second window never split and stale positions looked like
    ; gaps.
    global WWM_TilerOwned
    if WWM_TilerOwned.Has(hwnd)
        return false
    try {
        ; WS_POPUP (0x80000000) with no WS_CAPTION (0x00C00000) = borderless.
        style := WinGetStyle("ahk_id " hwnd)
        if !(style & 0x80000000)
            return false
        if (style & 0x00C00000)
            return false
        ; And it must actually cover its monitor.
        WinGetPos(&x, &y, &w, &h, "ahk_id " hwnd)
        cx := x + w / 2
        cy := y + h / 2
        Loop MonitorGetCount() {
            MonitorGet(A_Index, &l, &t, &r, &b)
            if cx >= l && cx < r && cy >= t && cy < b
                return x <= l + 2 && y <= t + 2 && x + w >= r - 2 && y + h >= b - 2
        }
    }
    return false
}

Tiling_IsMaximized(hwnd) {
    try return WinGetMinMax("ahk_id " hwnd) = 1
    catch
        return false
}

; Puts a single-window monitor into the real maximized state (Win+Up / the
; maximize caption button), rather than only resizing it to the monitor bounds.
Tiling_MaximizeSingle(hwnd) {
    global WWM_TilerOwned
    if !Tiling_IsMaximized(hwnd) {
        try WinMaximize("ahk_id " hwnd)
    }
    WWM_TilerOwned[hwnd] := true
}

Tiling_MonitorForRect(x, y, w, h) {
    cx := x + w / 2
    cy := y + h / 2
    Loop MonitorGetCount() {
        MonitorGet(A_Index, &l, &t, &r, &b)
        if cx >= l && cx < r && cy >= t && cy < b
            return A_Index
    }
    return MonitorGetPrimary()
}

Tiling_MonitorForWindow(hwnd) {
    if !hwnd || !WinExist("ahk_id " hwnd)
        return MonitorGetPrimary()
    try {
        WinGetPos(&x, &y, &w, &h, "ahk_id " hwnd)
        return Tiling_MonitorForRect(x, y, w, h)
    }
    return MonitorGetPrimary()
}

Tiling_ApplyOrder(desktop, monitor, windows) {
    global WWM_TileOrder
    key := desktop ":" monitor
    old := WWM_TileOrder.Has(key) ? WWM_TileOrder[key] : []

    result := []
    ; Keep previously known windows in their established order.
    for hwnd in old {
        if hwnd && WinExist("ahk_id " hwnd) && Tiling_Contains(windows, hwnd)
            result.Push(hwnd)
    }
    ; Then append newly seen windows, most recently activated last.
    for hwnd in windows {
        if !Tiling_Contains(result, hwnd)
            result.Push(hwnd)
    }
    WWM_TileOrder[key] := result
    return result
}

Tiling_Contains(list, value) {
    for item in list {
        if item = value
            return true
    }
    return false
}

; Master column plus an exactly-divided stack column. No gaps, no seams.
Tiling_LayoutMasterStack(windows, l, t, r, b) {
    global WWM_MasterRatio, WWM_MinTileWidth, WWM_MinTileHeight
    count := windows.Length
    if !count
        return

    width := r - l
    height := b - t
    if count = 1 {
        Tiling_Place(windows[1], l, t, width, height)
        return
    }

    masterW := Round(width * WWM_MasterRatio)
    ; Keep the master usable on small screens.
    masterW := Min(Max(masterW, WWM_MinTileWidth), width - WWM_MinTileWidth)
    stackX := l + masterW
    stackW := r - stackX

    Tiling_Place(windows[1], l, t, masterW, height)

    stackCount := count - 1
    Loop stackCount {
        idx := A_Index + 1
        ; Derive each slot from the shared boundary, not from accumulated
        ; rounded heights, so neighbours meet pixel-exactly.
        y1 := t + Round(height * (A_Index - 1) / stackCount)
        y2 := t + Round(height * A_Index / stackCount)
        Tiling_Place(windows[idx], stackX, y1, stackW, y2 - y1)
    }
}

; Visible frame of a window, ignoring Windows' invisible resize borders.
Tiling_VisibleFrame(hwnd) {
    rect := Buffer(16, 0)
    if DllCall("dwmapi\DwmGetWindowAttribute", "Ptr", hwnd, "Int", 9 ; DWMWA_EXTENDED_FRAME_BOUNDS
            , "Ptr", rect, "Int", 16, "Int") = 0 {
        x := NumGet(rect, 0, "Int")
        y := NumGet(rect, 4, "Int")
        r := NumGet(rect, 8, "Int")
        b := NumGet(rect, 12, "Int")
        if (r - x) > 0 && (b - y) > 0
            return {x: x, y: y, w: r - x, h: b - y}
    }
    try {
        WinGetPos(&x, &y, &w, &h, "ahk_id " hwnd)
        return {x: x, y: y, w: w, h: h}
    }
    return 0
}

Tiling_Place(hwnd, x, y, w, h) {
    global WWM_MinTileWidth, WWM_MinTileHeight, WWM_TilerOwned
    if w < WWM_MinTileWidth || h < WWM_MinTileHeight
        return
    try {
        ; True borderless fullscreen and maximized windows are never touched.
        if Tiling_IsFullscreen(hwnd)
            return

        ; Already exactly here? Do nothing. This is what stops the tiler from
        ; re-issuing SetWindowPos every tick.
        if Tiling_MatchesTarget(hwnd, x, y, w, h) {
            WWM_TilerOwned[hwnd] := true
            return
        }

        ; Windows adds an invisible resize border to the *classic* window rect.
        ; If the classic rect is set straight to the tile, the visible frame is
        ; inset by that border, which is the unwanted gap between neighbours.
        ; Expand the classic rect by the border so the visible frame lands on
        ; the requested tile.
        insets := Tiling_FrameInsets(hwnd)
        Tiling_MoveInert(hwnd
            , x - insets.l, y - insets.t
            , w + insets.l + insets.r, h + insets.t + insets.b)

        ; Measure the actual visible frame and correct any residual offset.
        ; This catches DPI rounding, cloaked-window DWM oddities, and windows
        ; whose border insets differ from the first measurement.
        Loop 3 {
            frame := Tiling_VisibleFrame(hwnd)
            if !frame
                break
            dx := x - frame.x
            dy := y - frame.y
            dw := w - frame.w
            dh := h - frame.h
            if Abs(dx) <= 1 && Abs(dy) <= 1 && Abs(dw) <= 1 && Abs(dh) <= 1
                break
            try {
                WinGetPos(&cx, &cy, &cw, &ch, "ahk_id " hwnd)
                Tiling_MoveInert(hwnd, cx + dx, cy + dy, cw + dw, ch + dh)
            }
        }
        WWM_TilerOwned[hwnd] := true
    }
}

; Difference between the classic window rect (what SetWindowPos positions) and
; the visible frame (what actually draws). Windows' invisible resize borders
; live in this difference. Size by the classic rect plus these insets so the
; visible frame fills the requested tile.
Tiling_FrameInsets(hwnd) {
    rect := Buffer(16, 0)
    if DllCall("dwmapi\DwmGetWindowAttribute", "Ptr", hwnd, "Int", 9 ; DWMWA_EXTENDED_FRAME_BOUNDS
            , "Ptr", rect, "Int", 16, "Int") = 0 {
        ex := NumGet(rect, 0, "Int")
        ey := NumGet(rect, 4, "Int")
        er := NumGet(rect, 8, "Int")
        eb := NumGet(rect, 12, "Int")
        try {
            WinGetPos(&cx, &cy, &cw, &ch, "ahk_id " hwnd)
            if cw > 0 && ch > 0 && (er - ex) > 0 && (eb - ey) > 0
                return {l: ex - cx, t: ey - cy, r: (cx + cw) - er, b: (cy + ch) - eb}
        }
    }
    return {l: 0, t: 0, r: 0, b: 0}
}

; Tolerant equality: DWM rounds frames to whole pixels, so allow 1px slop.
Tiling_MatchesTarget(hwnd, x, y, w, h) {
    frame := Tiling_VisibleFrame(hwnd)
    if !frame
        return false
    return Abs(frame.x - x) <= 1 && Abs(frame.y - y) <= 1
        && Abs(frame.w - w) <= 1 && Abs(frame.h - h) <= 1
}

; Moves a window without ever touching activation or z-order. WinMove raises
; the window and can steal foreground, which made the taskbar flash during
; workspace switches. SWP_NOACTIVATE|SWP_NOZORDER|SWP_NOOWNERZORDER keep the
; layout purely geometric.
Tiling_MoveInert(hwnd, x, y, w, h) {
    static SWP_NOZORDER := 0x0004, SWP_NOACTIVATE := 0x0010, SWP_NOOWNERZORDER := 0x0200
    DllCall("SetWindowPos", "Ptr", hwnd, "Ptr", 0, "Int", x, "Int", y
        , "Int", w, "Int", h
        , "UInt", SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER)
}

Tiling_ForgetWindow(hwnd) {
    global WWM_TileOrder, WWM_FloatingWindows, WWM_TilerOwned
    for key, order in WWM_TileOrder {
        for i, item in order {
            if item = hwnd {
                order.RemoveAt(i)
                break
            }
        }
    }
    ; Map.Delete throws if the key is absent, and Tiling_ForgetWindow is also
    ; called from the shell destroy hook for windows that were never floating
    ; or tiled, so only delete keys that exist.
    if WWM_FloatingWindows.Has(hwnd)
        WWM_FloatingWindows.Delete(hwnd)
    if WWM_TilerOwned.Has(hwnd)
        WWM_TilerOwned.Delete(hwnd)
}

Tiling_MoveFocused(direction) {
    global WWM_TileOrder
    hwnd := WWM_GetForegroundWindow()
    if !hwnd
        return
    monitor := Tiling_MonitorForWindow(hwnd)
    key := VD_Current() ":" monitor
    if !WWM_TileOrder.Has(key) {
        Tiling_TileCurrent(true)
        if !WWM_TileOrder.Has(key)
            return
    }
    order := WWM_TileOrder[key]
    idx := 0
    for i, item in order {
        if item = hwnd {
            idx := i
            break
        }
    }
    if !idx
        return

    ; H/L walk the stack; J/K move a window in or out of the master slot.
    target := idx
    switch direction {
        case "left", "up":
            target := Max(1, idx - 1)
        case "right", "down":
            target := Min(order.Length, idx + 1)
    }
    if target = idx
        return
    tmp := order[idx]
    order[idx] := order[target]
    order[target] := tmp
    WWM_TileOrder[key] := order
    Tiling_TileCurrent(true)
}

Tiling_ToggleFloat() {
    global WWM_FloatingWindows
    hwnd := WWM_GetForegroundWindow()
    if !hwnd || Tiling_IsExcluded(hwnd)
        return
    if WWM_FloatingWindows.Has(hwnd) {
        WWM_FloatingWindows.Delete(hwnd)
    } else {
        WWM_FloatingWindows[hwnd] := true
        ; A maximized window can't be resized, so restore it before floating.
        try if WinGetMinMax("ahk_id " hwnd) = 1
            WinRestore("ahk_id " hwnd)
    }
    Tiling_TileCurrent(true)
    Overlay_Show()
}

Tiling_SnapActive(direction) {
    global WWM_FloatingWindows
    hwnd := WWM_GetForegroundWindow()
    if !hwnd || Tiling_IsExcluded(hwnd)
        return
    monitor := Tiling_MonitorForWindow(hwnd)
    ; Snap within the work area so a visible taskbar is not overlapped.
    MonitorGetWorkArea(monitor, &l, &t, &r, &b)
    midX := l + Round((r - l) / 2)
    midY := t + Round((b - t) / 2)
    ; Snapped windows float until toggled back, matching i3's floating scratchpad.
    WWM_FloatingWindows[hwnd] := true
    try if WinGetMinMax("ahk_id " hwnd) = 1
        WinRestore("ahk_id " hwnd)
    switch direction {
        case "left":   Tiling_Place(hwnd, l, t, midX - l, b - t)
        case "right":  Tiling_Place(hwnd, midX, t, r - midX, b - t)
        case "top":    Tiling_Place(hwnd, l, t, r - l, midY - t)
        case "bottom": Tiling_Place(hwnd, l, midY, r - l, b - midY)
    }
}

Tiling_DescribeLayout() {
    monitor := Tiling_MonitorForWindow(WWM_GetForegroundWindow())
    count := Tiling_GetOrderedWindows(monitor).Length
    if count = 0
        return "no tiled windows"
    if count = 1
        return "fullscreen"
    return "master / stack (" count ")"
}
