global FocusBridge := Map()
global WWM_FocusHistory := Map()   ; desktop -> array of hwnd, oldest first
global WWM_SwitchingDesktop := false

; HRESULTs are signed 32-bit. DllCall hands back an unsigned value, so test the
; signed interpretation, otherwise every failure looks like success.
HRESULT_Succeeded(hr) {
    return ((hr & 0xFFFFFFFF) >> 31) = 0
}

FocusBridge_Init() {
    global FocusBridge, WWM_FocusBridgeEnabled
    if !WWM_FocusBridgeEnabled
        return
    dll := A_ScriptDir "\FocusBridge.dll"
    if !FileExist(dll)
        return ; WinActivate fallback is used; build the native bridge for robust focus.
    h := DllCall("LoadLibrary", "Str", dll, "Ptr")
    if !h
        return
    FocusBridge["module"] := h
    for name in ["PrepareWindowFocus", "CommitWindowFocus", "CancelWindowFocus"] {
        p := DllCall("GetProcAddress", "Ptr", h, "AStr", name, "Ptr")
        if !p
            throw Error("FocusBridge.dll is missing export: " name)
        FocusBridge[name] := p
    }
}

FocusBridge_Prepare(hwnd) {
    global FocusBridge
    if !FocusBridge.Has("PrepareWindowFocus")
        return false
    try return HRESULT_Succeeded(DllCall(FocusBridge["PrepareWindowFocus"], "Ptr", hwnd, "Int"))
    catch
        return false
}

FocusBridge_Commit() {
    global FocusBridge
    if !FocusBridge.Has("CommitWindowFocus")
        return false
    try return HRESULT_Succeeded(DllCall(FocusBridge["CommitWindowFocus"], "Int"))
    catch
        return false
}

FocusBridge_Cancel() {
    global FocusBridge
    if FocusBridge.Has("CancelWindowFocus")
        try DllCall(FocusBridge["CancelWindowFocus"], "Int")
}

VD_IsUsableWindow(hwnd) {
    try {
        if !hwnd || !WinExist("ahk_id " hwnd)
            return false
        if !(WinGetStyle("ahk_id " hwnd) & 0x10000000) ; WS_VISIBLE
            return false
        if WinGetMinMax("ahk_id " hwnd) = -1             ; minimized
            return false
        return true
    } catch {
        return false
    }
}

WWM_GetForegroundWindow() {
    ; Unlike WinGetID("A"), this safely returns 0 when no window is active.
    return DllCall("GetForegroundWindow", "Ptr")
}

; WinGetList only returns windows on the current virtual desktop. Some tasks
; (picking a pre-switch focus target, clearing a stranded window's taskbar
; flash) need every top-level window, including ones on other desktops.
global WWM_EnumWindows := []

WWM_EnumWindowsProc(hwnd, lParam) {
    global WWM_EnumWindows
    WWM_EnumWindows.Push(hwnd)
    return true
}

WWM_GetAllWindows() {
    global WWM_EnumWindows
    WWM_EnumWindows := []
    cb := CallbackCreate(WWM_EnumWindowsProc, "Fast", 2)
    if !cb
        return WWM_EnumWindows
    DllCall("EnumWindows", "Ptr", cb, "Ptr", 0)
    CallbackFree(cb)
    return WWM_EnumWindows
}

; Windows activates windows without letting AHK take foreground rights in some
; cases (elevated windows, another virtual desktop, SetFocus from the shell).
; Preferred path: ask the shell's IApplicationView to take focus (see
; native/FocusBridge). Fallback: plain WinActivate, which may flash the taskbar.
Focus_Activate(hwnd, desktop := -1) {
    global FocusBridge, WWM_SwitchingDesktop
    if !hwnd || !WinExist("ahk_id " hwnd)
        return false
    if WWM_GetForegroundWindow() = hwnd {
        Focus_ClearFlash(hwnd)
        return true
    }

    ; Never try to activate a window that lives on another desktop: Windows
    ; denies foreground rights and responds by flashing the taskbar button.
    ; Use GetWindowDesktopNumber rather than IsWindowOnCurrentVirtualDesktop
    ; because the latter can lag behind during a switch animation.
    expected := desktop >= 0 ? desktop : VD_Current()
    if VD_WindowDesktop(hwnd) != expected
        return false

    Focus_ClearFlash(hwnd)

    ; Preferred path: hand focus to the shell proxy, which is allowed to set the
    ; foreground window even when the caller cannot. During a desktop-switch
    ; animation GetViewForHwnd can transiently fail, so retry briefly before
    ; falling back to WinActivate (which is what flashes the taskbar).
    if FocusBridge.Has("PrepareWindowFocus") {
        ; During a desktop switch, keep trying for up to ~800ms: the shell may
        ; report E_ACCESSDENIED for the target until the switch animation has
        ; actually made it the current desktop.
        deadline := A_TickCount + (WWM_SwitchingDesktop ? 800 : 0)
        Loop {
            if FocusBridge_Prepare(hwnd) {
                ok := FocusBridge_Commit()
                Sleep(20)
                if ok && WWM_GetForegroundWindow() = hwnd {
                    Focus_ClearFlash(hwnd)
                    return true
                }
            }
            if !WWM_SwitchingDesktop || A_TickCount >= deadline
                break
            Sleep(40)
        }
    }

    try {
        if WinGetMinMax("ahk_id " hwnd) = -1
            WinRestore("ahk_id " hwnd)
    }

    ; Fallback: WinActivate can lose the foreground race and make the taskbar
    ; flash. Clear the foreground lock first, then clear any flash and verify
    ; the result instead of trusting WinActivate's return value.
    Focus_ReleaseForegroundLock()
    try WinActivate("ahk_id " hwnd)
    Sleep(40)
    if WWM_GetForegroundWindow() = hwnd {
        Focus_ClearFlash(hwnd)
        return true
    }
    Focus_ClearFlash(hwnd)
    return false
}

; Clears taskbar flashing on every window. Windows sometimes starts a flash on
; a stranded window a beat after the switch that triggered it, so this is the
; blunt, reliable way to stop the orange blink after a desktop change.
Focus_ClearAllFlashes() {
    for hwnd in WWM_GetAllWindows() {
        Focus_ClearFlash(hwnd)
    }
}

; Stops the orange taskbar blink Windows starts when a foreground request fails.
; FLASHWINFO (x64): UINT cbSize; <pad>; HWND hwnd; DWORD dwFlags; UINT uCount; DWORD dwTimeout;
Focus_ClearFlash(hwnd) {
    try {
        if !hwnd || !WinExist("ahk_id " hwnd)
            return
        info := Buffer(4 + A_PtrSize + 16, 0)
        NumPut("UInt", info.Size, info, 0)          ; cbSize
        NumPut("Ptr", hwnd, info, 8)                ; hwnd
        NumPut("UInt", 0, info, 8 + A_PtrSize)      ; dwFlags = FLASHW_STOP
        DllCall("user32\FlashWindowEx", "Ptr", info)
    }
}

; The classic Alt-key foreground hack. A simulated Alt press/release clears
; Windows' foreground lock, which otherwise makes SetFocus fail when the
; current window is actively receiving input (e.g. while typing in a terminal).
Focus_ReleaseForegroundLock() {
    DllCall("user32\keybd_event", "UChar", 0x12, "UChar", 0, "UInt", 0, "UInt", 0)
    DllCall("user32\keybd_event", "UChar", 0x12, "UChar", 0, "UInt", 2, "UInt", 0)
}

; Focus is restored after a desktop switch by WindowManager_OnDesktopChanged,
; but only after the switch animation has settled. Activating during the switch
; races the shell, and losing that race is what flashes the taskbar button.

Focus_Record(desktop, hwnd) {
    global WWM_FocusHistory
    if !hwnd
        return
    list := WWM_FocusHistory.Has(desktop) ? WWM_FocusHistory[desktop] : []
    newList := []
    for item in list {
        if item != hwnd
            newList.Push(item)
    }
    newList.Push(hwnd)
    ; Keep the history bounded.
    while newList.Length > 32
        newList.RemoveAt(1)
    WWM_FocusHistory[desktop] := newList
}

; Directional focus follows the tiler's window order so that left/right and
; up/down are exact inverses of each other, i3-style. When the foreground
; window isn't part of the tile order (nothing focused, or a floating window),
; it falls back to spatial nearest-neighbour search.
Focus_Directional(direction) {
    desktop := VD_Current()
    active := WWM_GetForegroundWindow()
    order := Tiling_GetOrderedWindows()

    if !Focus_MoveInOrder(order, active, direction)
        Focus_Spatial(direction)

    Overlay_Show()
}

Focus_MoveInOrder(order, active, direction) {
    if order.Length < 2
        return true ; nothing to do; don't fall through to spatial search
    idx := 0
    for i, hwnd in order {
        if hwnd = active {
            idx := i
            break
        }
    }
    if !idx
        return false

    ; left/up = previous tile, right/down = next tile, wrapping at the ends.
    step := (direction = "left" || direction = "up") ? -1 : 1
    target := idx + step
    if target < 1
        target := order.Length
    else if target > order.Length
        target := 1
    if target = idx
        return true
    return Focus_Activate(order[target])
}

Focus_Spatial(direction) {
    active := WWM_GetForegroundWindow()
    if !active || !WinExist("ahk_id " active)
        return false
    frame := Tiling_VisibleFrame(active)
    if !frame
        return false
    acx := frame.x + frame.w / 2
    acy := frame.y + frame.h / 2

    best := 0
    bestScore := 1e100
    for hwnd in WinGetList() {
        if hwnd = active || !VD_IsUsableWindow(hwnd)
            continue
        if Tiling_IsExcluded(hwnd) || !Tiling_IsWindowOnCurrentDesktop(hwnd)
            continue
        f := Tiling_VisibleFrame(hwnd)
        if !f
            continue
        dx := (f.x + f.w / 2) - acx
        dy := (f.y + f.h / 2) - acy
        valid := false
        switch direction {
            case "left":  valid := dx < -4
            case "right": valid := dx > 4
            case "up":    valid := dy < -4
            case "down":  valid := dy > 4
        }
        if !valid
            continue
        ; Weight the off-axis distance less so diagonal neighbours still win.
        score := (direction = "left" || direction = "right")
            ? Abs(dx) + Abs(dy) * 1.8
            : Abs(dy) + Abs(dx) * 1.8
        if score < bestScore {
            bestScore := score
            best := hwnd
        }
    }
    if !best
        return false
    return Focus_Activate(best)
}

; Alt+Tab-like cycling through the per-desktop focus history.
Focus_CycleBack() {
    desktop := VD_Current()
    if !WWM_FocusHistory.Has(desktop)
        return
    list := WWM_FocusHistory[desktop]
    active := WWM_GetForegroundWindow()
    Loop list.Length {
        target := list[list.Length - A_Index + 1]
        if target && target != active && WinExist("ahk_id " target)
            && Tiling_IsWindowOnCurrentDesktop(target) {
            Focus_Activate(target)
            return
        }
    }
}
