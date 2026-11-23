global WWM_DesktopPollMs := 250
global WWM_LastDesktop := -1
global WWM_LastFocused := 0
global WWM_PreSwitchForeground := 0
global WWM_SwitchGeneration := 0
global WWM_ScheduledGeneration := 0

WindowManager_Init() {
    global WWM_LastDesktop, WWM_DesktopPollMs
    WWM_LastDesktop := VD_Current()
    WindowManager_ApplyLockPolicy()

    ; Tiling: only on a slow timer, exactly like the original. Fast polling was
    ; not needed and kept repositioning windows.
    SetTimer(Tiling_TileCurrent, WWM_TileIntervalMs)

    ; VirtualDesktopAccessor has no change callback, so poll for desktop switches.
    SetTimer(WindowManager_Poll, WWM_DesktopPollMs)
    SetTimer(WindowManager_TrackFocus, 200)
}

WindowManager_Poll() {
    global WWM_LastDesktop
    current := VD_Current()
    if current = WWM_LastDesktop
        return
    ; Shell-initiated switches (e.g. Win+Tab) have no hotkey path, so the poll
    ; is the fallback detector. Hotkey switches call this same hook directly
    ; from VD_SwitchTo.
    WindowManager_OnSwitchInitiated(WWM_GetForegroundWindow())
}

; Called as soon as a desktop switch is known (hotkey path or poll fallback).
; preFg is the window that had focus before the switch; its taskbar button is
; what Windows flashes while it is stranded on the old desktop.
WindowManager_OnSwitchInitiated(preFg, targetDesktop := -1) {
    global WWM_LastDesktop, WWM_SwitchingDesktop, WWM_PreSwitchForeground
    global WWM_SwitchGeneration, WWM_ScheduledGeneration
    WWM_LastDesktop := targetDesktop >= 0 ? targetDesktop : VD_Current()
    WWM_SwitchingDesktop := true
    WWM_PreSwitchForeground := preFg
    WWM_SwitchGeneration += 1
    WWM_ScheduledGeneration := WWM_SwitchGeneration
    ; Stop any stranded window from blinking immediately, and again after the
    ; switch animation has had a chance to start a fresh flash.
    Focus_ClearAllFlashes()
    SetTimer(Focus_ClearAllFlashes, -350)
    ; Focus and re-tile the incoming desktop as soon as the switch is known.
    ; WindowManager_FocusCurrentDesktop retries through the animation, so a
    ; short delay is enough.
    SetTimer(WindowManager_OnDesktopChanged, -30)
}

; Runs once, shortly after a desktop switch, once the shell has had time to
; finish the switch animation and report windows on the new desktop.
WindowManager_OnDesktopChanged() {
    global WWM_SwitchingDesktop, WWM_PreSwitchForeground
    global WWM_SwitchGeneration, WWM_ScheduledGeneration
    gen := WWM_ScheduledGeneration
    if gen != WWM_SwitchGeneration
        return
    WindowManager_FocusCurrentDesktop()
    Tiling_TileCurrent(true)
    ; If a newer switch happened while we were focusing/tiling, leave the
    ; cleanup to that switch's settle pass.
    if gen = WWM_SwitchGeneration {
        WWM_SwitchingDesktop := false
        if WWM_PreSwitchForeground
            Focus_ClearFlash(WWM_PreSwitchForeground)
        Focus_ClearAllFlashes()
        WWM_PreSwitchForeground := 0
    }
}

; Activates a window belonging to the current desktop. Uses the COM focus
; bridge first (no taskbar flash); falls back to WinActivate.
WindowManager_FocusCurrentDesktop() {
    ; Wait for the desktop switch to finish and for windows to report as being
    ; on this desktop. Keep retrying briefly: the first attempt can be too early
    ; while the shell still reports the old desktop, and giving up immediately
    ; is what leaves the taskbar focused.
    deadline := A_TickCount + 800
    target := 0
    Loop {
        fg := WWM_GetForegroundWindow()
        ; Only accept a real window as already-focused. The taskbar is also on
        ; the current desktop, and treating it as valid is what left the
        ; taskbar focused after a switch.
        if fg && VD_IsUsableWindow(fg) && VD_WindowDesktop(fg) = VD_Current()
            && !Tiling_IsExcluded(fg) {
            Focus_ClearFlash(fg)
            return
        }
        if !target || !WinExist("ahk_id " target) {
            target := WindowManager_PickFocusTarget()
        }
        if target && Focus_Activate(target)
            return
        if A_TickCount >= deadline
            break
        Sleep(40)
    }
}

; Chooses the best window to focus on the current desktop.
WindowManager_PickFocusTarget() {
    return WindowManager_PickFocusTargetForDesktop(VD_Current())
}

; Chooses a focus target on an arbitrary desktop (used before switching).
WindowManager_PickFocusTargetForDesktop(desktop) {
    global WWM_FocusHistory

    ; Prefer the most recently focused window on that desktop.
    if WWM_FocusHistory.Has(desktop) {
        list := WWM_FocusHistory[desktop]
        Loop list.Length {
            hwnd := list[list.Length - A_Index + 1]
            if hwnd && WinExist("ahk_id " hwnd)
                && VD_WindowDesktop(hwnd) = desktop
                && VD_IsUsableWindow(hwnd)
                && !Tiling_IsExcluded(hwnd)
                return hwnd
        }
    }

    ; Otherwise any usable window on that desktop. WWM_GetAllWindows is used
    ; because the target desktop is not the current one yet, so WinGetList
    ; (which only returns current-desktop windows) would never find it.
    for hwnd in WWM_GetAllWindows() {
        if !VD_IsUsableWindow(hwnd)
            continue
        if VD_WindowDesktop(hwnd) != desktop
            continue
        if Tiling_IsExcluded(hwnd)
            continue
        return hwnd
    }
    return 0
}

WindowManager_TrackFocus() {
    global WWM_LastFocused
    hwnd := WWM_GetForegroundWindow()
    if !hwnd || hwnd = WWM_LastFocused
        return
    if Tiling_IsExcluded(hwnd)
        return
    WWM_LastFocused := hwnd
    Focus_Record(VD_Current(), hwnd)
}

WindowManager_ToggleMaximize() {
    hwnd := WWM_GetForegroundWindow()
    if !hwnd || Tiling_IsExcluded(hwnd)
        return
    try {
        if WinGetMinMax("ahk_id " hwnd) = 1
            WinRestore("ahk_id " hwnd)
        else
            WinMaximize("ahk_id " hwnd)
    }
    SetTimer(Tiling_TileCurrent, -50)
}

; --- Optional: free up Win+L ------------------------------------------------
; Winlogon can win the race against a hotkey for Win+L. When the user opts in
; with WWM_DisableWinLock, set the documented per-user policy that removes the
; OS binding, and restore it when the script exits.

WWM_LockPolicy_Set(disabled) {
    static key := "HKCU\Software\Microsoft\Windows\CurrentVersion\Policies\System"
    try {
        if disabled
            RegWrite(1, "REG_DWORD", key, "DisableLockWorkstation")
        else
            RegDelete(key, "DisableLockWorkstation")
        return true
    } catch {
        return false
    }
}

WindowManager_ApplyLockPolicy() {
    global WWM_DisableWinLock
    if !WWM_DisableWinLock
        return
    if WWM_LockPolicy_Set(true)
        OnExit(WindowManager_RestoreLockPolicy)
}

WindowManager_RestoreLockPolicy(*) {
    WWM_LockPolicy_Set(false)
}
