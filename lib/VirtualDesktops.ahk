; Virtual desktop bridge around Ciantic's VirtualDesktopAccessor.dll.
;
; Desktop indexes are zero-based here (the DLL's convention). Convert to a
; one-based label only at the display layer.

global VDA := Map()

VDA_Init() {
    global VDA
    dll := A_ScriptDir "\VirtualDesktopAccessor.dll"
    if !FileExist(dll)
        throw Error("VirtualDesktopAccessor.dll was not found beside WWM.ahk.`n"
            . "Download the current x64 build from Ciantic/VirtualDesktopAccessor releases.")

    h := DllCall("LoadLibrary", "Str", dll, "Ptr")
    if !h
        throw Error("VirtualDesktopAccessor.dll could not be loaded. Make sure this is"
            . " running under AutoHotkey v2 x64 and that the DLL matches your Windows build.")

    VDA["module"] := h
    ; Every export used below must resolve, otherwise fail loudly at startup.
    for name in ["GetCurrentDesktopNumber", "GetDesktopCount", "GoToDesktopNumber",
                 "IsWindowOnCurrentVirtualDesktop", "MoveWindowToDesktopNumber",
                 "CreateDesktop", "GetWindowDesktopNumber"] {
        p := DllCall("GetProcAddress", "Ptr", h, "AStr", name, "Ptr")
        if !p
            throw Error("VirtualDesktopAccessor.dll is missing export: " name
                . "`nThe DLL is older than this Windows build or is the wrong architecture.")
        VDA[name] := p
    }
}

VD_Call(name, args*) {
    global VDA
    if !VDA.Has(name)
        throw Error("VD_Call: VirtualDesktopAccessor.dll is not initialised (missing " name ").")
    return DllCall(VDA[name], args*)
}

VD_Current() {
    try return VD_Call("GetCurrentDesktopNumber", "Int")
    catch
        return 0
}

VD_Count() {
    try return VD_Call("GetDesktopCount", "Int")
    catch
        return 1
}

; Zero-based desktop index a window currently lives on, or -1 if unknown.
VD_WindowDesktop(hwnd) {
    try return VD_Call("GetWindowDesktopNumber", "Ptr", hwnd, "Int")
    catch
        return -1
}

; One-based label for the overlay. Desktops are unnamed: "1", "2", "3", ...
VD_Label(index) {
    return index + 1
}

VD_SwitchTo(index) {
    count := VD_Count()
    if index < 0 || index >= count
        return false
    if index = VD_Current()
        return true

    preFg := WWM_GetForegroundWindow()
    target := WindowManager_PickFocusTargetForDesktop(index)

    ; FocusBridge cannot prepare a window while it is still on another virtual
    ; desktop (GetViewForHwnd returns E_ACCESSDENIED), so switch first and then
    ; let WindowManager_OnDesktopChanged focus the target once the switch has
    ; settled. Doing it here too caused two concurrent FocusBridge users and
    ; corrupted the bridge's global COM state.
    ok := false
    try {
        ; VirtualDesktopAccessor returns a boolean-style int: 1 = success,
        ; -1 = failure. Do NOT test for 0 (that is not the success value and
        ; made every switch look like a failure, which is why the hotkey path
        ; silently fell back to the slow desktop poll).
        ok := VD_Call("GoToDesktopNumber", "Int", index, "Int") = 1
    }
    if !ok
        return false

    if preFg
        Focus_ClearFlash(preFg)
    WindowManager_OnSwitchInitiated(preFg, index)
    return true
}

VD_SwitchRelative(delta) {
    count := VD_Count()
    if count < 1
        return
    ; Add count first because AHK's Mod keeps the sign of the dividend, so a
    ; negative desktop index would not wrap around to the last workspace.
    target := Mod(VD_Current() + delta + count, count)
    VD_SwitchTo(target)
}

VD_CreateAndSwitch() {
    count := VD_Count()
    try VD_Call("CreateDesktop", "Int")
    catch
        return false
    ; Give the shell a moment to register the new desktop, then switch to it.
    ; CreateDesktop appends, so the new desktop is at index == previous count.
    Sleep(120)
    return VD_SwitchTo(count)
}

VD_MoveActiveTo(index) {
    hwnd := WWM_GetForegroundWindow()
    if !hwnd
        return false
    return VD_MoveWindowTo(hwnd, index)
}

VD_MoveWindowTo(hwnd, index) {
    if index < 0 || index >= VD_Count()
        return false
    try {
        if !VD_IsUsableWindow(hwnd)
            return false
        ; 1 = success, -1 = failure. (The DLL does not return an HRESULT here;
        ; testing = 0 made every move look like it failed, so the moved window
        ; was never tiled on its new desktop until the user switched to it.)
        ok := VD_Call("MoveWindowToDesktopNumber", "Ptr", hwnd, "Int", index, "Int") = 1
        if ok {
            ; The window now belongs to the target desktop. Forget its old slot
            ; so the target desktop's order picks it up cleanly, then re-tile
            ; both the desktop we are still on (it lost a window) and the
            ; destination desktop (it gained one).
            Tiling_ForgetWindow(hwnd)
            Tiling_TileCurrent(true)
            if index != VD_Current() {
                Tiling_TileDesktop(index, true)
                ; Safety net: if a future DLL build ever reports the old
                ; desktop for a moment after the move, retile the destination
                ; shortly afterwards so the moved window does not stay floating.
                Tiling_ScheduleDesktopRetile(index)
            }
        }
        return ok
    } catch {
        return false
    }
}
