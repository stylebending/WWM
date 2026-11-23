; Windows Workspace Manager settings
;
; Desktops are unnamed: the overlay shows plain 1-based numbers ("1", "2", ...).
; Desktop indexes are zero-based internally.

global A_BrowserPath := "brave.exe"

; Master window width as a fraction of the monitor work area. 0.5 = an even
; 50/50 split (Hyprland's master default).
global WWM_MasterRatio := 0.5

; Tiling is fully gapless. There is no gap setting on purpose: every window
; edge is computed from shared boundaries so no seams appear between tiles.
global WWM_TileIntervalMs := 250
global WWM_MinTileWidth := 160
global WWM_MinTileHeight := 120

global WWM_OverlayDurationMs := 1100
global WWM_OverlayWidth := 390
global WWM_OverlayHeight := 92
global WWM_OverlayTopOffset := 42

global WWM_FocusBridgeEnabled := true
global WWM_AutoTileEnabled := true

; Add executable names here to keep apps floating (case-insensitive).
global WWM_FloatProcessNames := Map(
    "ApplicationFrameHost.exe", true,
    "ShellExperienceHost.exe", true,
    "SearchHost.exe", true,
    "StartMenuExperienceHost.exe", true,
    "TextInputHost.exe", true
)

; Window classes that are never tiled.
global WWM_FloatClassNames := Map(
    "Shell_TrayWnd", true,
    "Shell_SecondaryTrayWnd", true,
    "Progman", true,
    "WorkerW", true,
    "Windows.UI.Core.CoreWindow", true,
    "XamlExplorerHostIslandWindow", true,
    "MultitaskingViewFrame", true,
    "ForegroundStaging", true
)

; Win+L is bound to focus-right. If Win+L still locks the session on your
; machine (Winlogon can beat the hotkey), set this to true: Windows Workspace
; Manager then disables the OS Win+L binding for the current user and restores
; it on exit. Lock remains available as Win+Alt+L.
global WWM_DisableWinLock := false
