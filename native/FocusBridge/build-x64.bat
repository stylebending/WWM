@echo off
setlocal
where cl.exe >nul 2>nul
if errorlevel 1 (
  echo Run this from "x64 Native Tools Command Prompt for VS".
  exit /b 1
)
cl.exe /nologo /LD /EHsc /std:c++17 /O2 FocusBridge.cpp ole32.lib user32.lib /link /OUT:FocusBridge.dll
if errorlevel 1 exit /b 1
copy /Y FocusBridge.dll ..\..\FocusBridge.dll
echo Built FocusBridge.dll in the WWM root.
