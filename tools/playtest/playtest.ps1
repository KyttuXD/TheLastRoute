# LastRoute Playtest: checks the nightly Windows build, downloads it when it has changed, then starts it.
# Run by the desktop shortcut (see install-playtest.ps1). Needs the GitHub CLI (gh) signed in to KyttuXD.
# -NoLaunch only updates the local copy.
param([switch]$NoLaunch)

$ErrorActionPreference = 'Stop'
$Repo = 'KyttuXD/TheLastRoute'
$Asset = 'last_route-windows-x86_64.exe'
$Keep = 3

$Root = Join-Path $env:LOCALAPPDATA 'LastRoutePlaytest'
$Builds = Join-Path $Root 'builds'
$LogPath = Join-Path $Root 'playtest.log'
New-Item -ItemType Directory -Force -Path $Builds | Out-Null

function Write-Log([string]$message) {
    Add-Content -Path $LogPath -Value ('{0} {1}' -f (Get-Date -Format 's'), $message)
}

function Get-LocalBuild {
    # the newest build folder that has a game exe in it
    Get-ChildItem $Builds -Directory |
        Sort-Object Name -Descending |
        Where-Object { Test-Path (Join-Path $_.FullName 'last_route.exe') } |
        Select-Object -First 1
}

$current = $null
try {
    $gh = (Get-Command gh -ErrorAction Stop).Source
    $release = & $gh release view nightly --repo $Repo --json assets 2>$null | ConvertFrom-Json
    $remote = $release.assets | Where-Object { $_.name -eq $Asset } | Select-Object -First 1
    if (-not $remote) { throw "$Asset is not in the nightly release" }

    # the build's id is when its file was uploaded, so a new nightly always gets a new folder
    $stamp = ([datetime]$remote.updatedAt).ToUniversalTime().ToString('yyyyMMdd-HHmmss')
    $dir = Join-Path $Builds $stamp
    $exe = Join-Path $dir 'last_route.exe'
    if (Test-Path $exe) {
        Write-Log "build $stamp is current"
    } else {
        Write-Log "build ${stamp}: downloading"
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
        & $gh release download nightly --repo $Repo --pattern $Asset --dir $dir --clobber 2>$null
        $downloaded = Join-Path $dir $Asset
        if (-not (Test-Path $downloaded)) { throw "download of $Asset failed" }
        Move-Item -Force $downloaded $exe
        Write-Log "build ${stamp}: ready"
    }
    $current = $stamp
} catch {
    Write-Log "update check failed: $($_.Exception.Message)"
}

if (-not $current) {
    # offline or gh not ready: play the newest build already downloaded
    $local = Get-LocalBuild
    if (-not $local) {
        Write-Log 'no build is available and none could be downloaded'
        exit 1
    }
    $current = $local.Name
    Write-Log "using the local build $current"
}

$exe = Join-Path (Join-Path $Builds $current) 'last_route.exe'

# keep the newest few builds; the one about to run is never removed
Get-ChildItem $Builds -Directory |
    Sort-Object Name -Descending |
    Select-Object -Skip $Keep |
    Where-Object { $_.Name -ne $current } |
    ForEach-Object { Remove-Item -Recurse -Force $_.FullName -ErrorAction SilentlyContinue }

if (-not $NoLaunch) {
    Start-Process -FilePath $exe -WorkingDirectory (Split-Path $exe)
    Write-Log "started build $current"
}
