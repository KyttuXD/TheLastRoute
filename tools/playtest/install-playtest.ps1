# Sets up the LastRoute playtest on this PC: copies playtest.ps1 to %LOCALAPPDATA%\LastRoutePlaytest, runs it
# once to fetch the newest nightly, and puts a "LastRoute Playtest.bat" launcher on the desktop.
# Run from this folder:  powershell -ExecutionPolicy Bypass -File install-playtest.ps1
$ErrorActionPreference = 'Stop'
$Root = Join-Path $env:LOCALAPPDATA 'LastRoutePlaytest'
New-Item -ItemType Directory -Force -Path $Root | Out-Null
$script = Join-Path $Root 'playtest.ps1'
Copy-Item -Force (Join-Path $PSScriptRoot 'playtest.ps1') $script

# first fetch (no launch)
powershell -NoProfile -ExecutionPolicy Bypass -File $script -NoLaunch

# the launcher: start PowerShell hidden, so the batch window closes straight away
$desktop = [Environment]::GetFolderPath('Desktop')
$bat = Join-Path $desktop 'LastRoute Playtest.bat'
$line = 'start "" powershell -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File "%LOCALAPPDATA%\LastRoutePlaytest\playtest.ps1"'
Set-Content -Path $bat -Value @('@echo off', $line) -Encoding ASCII
Write-Output "Playtest installed. Launcher: $bat"

