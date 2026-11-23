global WWM_OverlayGui := 0
global WWM_OverlayText := 0
global WWM_OverlayDetail := 0
global WWM_OverlayHwnd := 0

Overlay_Init() {
    global WWM_OverlayGui, WWM_OverlayText, WWM_OverlayDetail, WWM_OverlayHwnd
    global WWM_OverlayWidth, WWM_OverlayHeight

    WWM_OverlayGui := Gui("+AlwaysOnTop -Caption +ToolWindow +E0x08000000")
    WWM_OverlayGui.BackColor := "0F172A"
    WWM_OverlayGui.MarginX := 18
    WWM_OverlayGui.MarginY := 10
    WWM_OverlayGui.SetFont("s9 c64748B", "Segoe UI")
    WWM_OverlayGui.AddText("w" (WWM_OverlayWidth - 36) " h16", "WORKSPACE")
    WWM_OverlayGui.SetFont("s24 cF8FAFC Bold", "Segoe UI")
    WWM_OverlayText := WWM_OverlayGui.AddText("w" (WWM_OverlayWidth - 36) " h38", "")
    WWM_OverlayGui.SetFont("s9 c94A3B8", "Segoe UI")
    WWM_OverlayDetail := WWM_OverlayGui.AddText("w" (WWM_OverlayWidth - 36) " h16", "")

    ; NOTE: .Hwnd is 0 until the GUI is first shown, so it is captured in
    ; Overlay_Show() instead of here. Capturing it here left WWM_OverlayHwnd = 0,
    ; which silently defeated the overlay exclusion in Tiling_IsExcluded().
}

Overlay_Show() {
    global WWM_OverlayGui, WWM_OverlayText, WWM_OverlayDetail, WWM_OverlayHwnd
    global WWM_OverlayWidth, WWM_OverlayHeight, WWM_OverlayTopOffset, WWM_OverlayDurationMs
    if !IsObject(WWM_OverlayGui)
        return

    current := VD_Current()
    count := VD_Count()
    WWM_OverlayText.Text := VD_Label(current)
    WWM_OverlayDetail.Text := "of " count "   ·   " Tiling_DescribeLayout()

    ; Centre on the monitor that owns the foreground window, not always primary.
    monitor := Tiling_MonitorForWindow(WWM_GetForegroundWindow())
    MonitorGetWorkArea(monitor, &left, &top, &right, &bottom)
    x := left + ((right - left) - WWM_OverlayWidth) // 2
    y := top + WWM_OverlayTopOffset

    ; "NA" = no-activate, so the overlay never takes focus away from the window
    ; you just switched to (which is what made the taskbar end up focused).
    WWM_OverlayGui.Show("NA x" x " y" y " w" WWM_OverlayWidth " h" WWM_OverlayHeight)
    WWM_OverlayHwnd := WWM_OverlayGui.Hwnd

    ; Repeated calls restart the one-shot hide timer.
    SetTimer(Overlay_Hide, 0)
    SetTimer(Overlay_Hide, -WWM_OverlayDurationMs)
}

Overlay_Hide() {
    global WWM_OverlayGui
    if IsObject(WWM_OverlayGui)
        WWM_OverlayGui.Hide()
}
