@echo off
setlocal
cd /d "%~dp0"
set PATH=%USERPROFILE%\.cargo\bin;%PATH%
echo Building latest local last_route (debug)...
cargo build --bin last_route
if errorlevel 1 (
  echo BUILD FAILED - close any game window that uses the build output, then run this again.
  pause
  exit /b 1
)
rem Launch a timestamped copy, so a game still open from an earlier run never blocks the next build.
for /f %%i in ('powershell -NoProfile -Command "Get-Date -Format yyyyMMdd-HHmmss"') do set STAMP=%%i
if not exist "target\playtest" mkdir "target\playtest"
copy /y "target\debug\last_route.exe" "target\playtest\last_route-%STAMP%.exe" >nul
start "" "%~dp0target\playtest\last_route-%STAMP%.exe"
