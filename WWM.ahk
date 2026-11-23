#Requires AutoHotkey v2.0
#SingleInstance Force
#Warn LocalSameAsGlobal, Off
#Warn Unreachable, Off
#UseHook true

; Ensure AutoHotkey and DWM use the same coordinate space (physical pixels).
; If the process is DPI-unaware, WinGetPos/SetWindowPos are virtualized while
; DwmGetWindowAttribute is not, which makes border compensation wrong and
; produces gaps. This call is harmless if awareness is already set.
try DllCall("shcore\SetProcessDpiAwareness", "Int", 2, "Int")

; #SingleInstance Force normally handles this, but a detached launch can slip
; through. Multiple copies all poll, re-tile and compete for foreground focus,
; which is a major cause of stray blinking. Refuse to be the second copy.
if WWM_OtherInstanceExists() {
    MsgBox("Windows Workspace Manager is already running.`n`nClose the existing copy first "
        . "(Win+Ctrl+Esc there, or end AutoHotkey64.exe in Task Manager).",
        "Windows Workspace Manager", "Iconi")
    ExitApp()
}

; True when another AutoHotkey process is running this exact script.
WWM_OtherInstanceExists() {
    me := DllCall("GetCurrentProcessId", "UInt")
    try {
        for proc in ComObjGet("winmgmts:").ExecQuery(
            "SELECT ProcessId, CommandLine FROM Win32_Process WHERE Name='AutoHotkey64.exe'"
            . " OR Name='AutoHotkey32.exe'") {
            if (proc.ProcessId = me)
                continue
            try {
                if InStr(proc.CommandLine, A_ScriptFullPath)
                    return true
            }
        }
    } catch {
        return false
    }
    return false
}

#Include config\settings.ahk
#Include lib\VirtualDesktops.ahk
#Include lib\Focus.ahk
#Include lib\Tiling.ahk
#Include lib\Overlay.ahk
#Include lib\WindowManager.ahk

try {
    VDA_Init()
    FocusBridge_Init()
    Overlay_Init()
    Tiling_Init()
    WindowManager_Init()
    Overlay_Show()
} catch as err {
    MsgBox("Windows Workspace Manager could not start.`n`n" err.Message, "Windows Workspace Manager", "Iconx")
    ExitApp()
}

; --- Workspaces -------------------------------------------------------------
; Kept free of $/#UseHook overrides so AutoHotkey can use RegisterHotKey.
#1::VD_SwitchTo(0)
#2::VD_SwitchTo(1)
#3::VD_SwitchTo(2)
#4::VD_SwitchTo(3)
#5::VD_SwitchTo(4)
#6::VD_SwitchTo(5)
#7::VD_SwitchTo(6)
#8::VD_SwitchTo(7)
#9::VD_SwitchTo(8)
#0::VD_SwitchTo(9)

; Move the focused window to another workspace without following it.
#+1::VD_MoveActiveTo(0)
#+2::VD_MoveActiveTo(1)
#+3::VD_MoveActiveTo(2)
#+4::VD_MoveActiveTo(3)
#+5::VD_MoveActiveTo(4)
#+6::VD_MoveActiveTo(5)
#+7::VD_MoveActiveTo(6)
#+8::VD_MoveActiveTo(7)
#+9::VD_MoveActiveTo(8)
#+0::VD_MoveActiveTo(9)

#^Left::VD_SwitchRelative(-1)
#^Right::VD_SwitchRelative(1)
#^n::VD_CreateAndSwitch()

; --- Focus ------------------------------------------------------------------
; Win+H / Win+J / Win+K / Win+L walk the tiles.
;
; Win+L is unreliable to intercept: Winlogon handles the lock on a path that
; can win the race against the hotkey, which is why it kept locking even with
; the hotkey registered. `$` forces the keyboard hook, which is the best this
; script can do. If Win+L STILL locks on your machine, set
; WWM_DisableWinLock := true in config\settings.ahk: Windows Workspace Manager
; then disables the OS Win+L binding for the current user and restores it on exit.
#h::Focus_Directional("left")
#j::Focus_Directional("down")
#k::Focus_Directional("up")
$#l::Focus_Directional("right")

; Cycle focus backwards through the per-desktop history.
#!h::Focus_CycleBack()

; --- Window management ------------------------------------------------------
#+h::Tiling_MoveFocused("left")
#+j::Tiling_MoveFocused("down")
#+k::Tiling_MoveFocused("up")
#+l::Tiling_MoveFocused("right")

#^h::Tiling_SnapActive("left")
#^l::Tiling_SnapActive("right")
#^k::Tiling_SnapActive("top")
#^j::Tiling_SnapActive("bottom")
#+Space::Tiling_ToggleFloat()
#Space::Overlay_Show()

; --- Applications -----------------------------------------------------------
#b::Run(A_BrowserPath)
#e::Run("explorer.exe")
#t::Run("wt.exe")
#n::Run("notepad.exe")
#q::WinClose("A")
#f::WindowManager_ToggleMaximize()

; Lock the session. Win+L is taken by focus-right, so this moves here.
#!l::DllCall("user32\LockWorkStation")

; --- Script control ---------------------------------------------------------
#Tab::Overlay_Show()
#^r::Reload()
#^Escape::ExitApp()
