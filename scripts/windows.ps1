<#
.SYNOPSIS
    Build, install or uninstall Katana Desktop on Windows (the equivalent of `make`).

.DESCRIPTION
    No switch      build target\release\katana-desktop.exe (in a source checkout)
    -Install       install for the current user: %LOCALAPPDATA%\Programs\Katana Desktop,
                   a Start menu shortcut and an "Apps & features" entry. From a release zip it
                   installs the bundled .exe; in a source checkout it builds first.
    -Uninstall     remove all of that again (your data in %LOCALAPPDATA%\katana-desktop is kept)

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\windows.ps1 -Install
#>
param([switch]$Install, [switch]$Uninstall)

$ErrorActionPreference = 'Stop'
$AppName      = 'Katana Desktop'
$ExeName      = 'katana-desktop.exe'
$InstallDir   = Join-Path $env:LOCALAPPDATA "Programs\$AppName"
$Shortcut     = Join-Path ([Environment]::GetFolderPath('Programs')) "$AppName.lnk"
$UninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\KatanaDesktop'

function Stop-App {
    Get-Process -Name 'katana-desktop' -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Milliseconds 300
}

if ($Uninstall) {
    Stop-App
    Remove-Item $Shortcut -Force -ErrorAction SilentlyContinue
    Remove-Item $UninstallKey -Recurse -Force -ErrorAction SilentlyContinue
    Set-Location $env:TEMP  # we may be running from inside the install folder
    Remove-Item $InstallDir -Recurse -Force -ErrorAction SilentlyContinue
    Write-Host "Uninstalled $AppName. Your data is kept in $env:LOCALAPPDATA\katana-desktop"
    exit 0
}

# A release zip ships the .exe next to this script: install that. In a source checkout, build first.
$Prebuilt = Join-Path $PSScriptRoot $ExeName
if (Test-Path $Prebuilt) {
    $Built = $Prebuilt
} else {
    $Root = Split-Path $PSScriptRoot -Parent
    if (-not (Test-Path (Join-Path $Root 'Cargo.toml'))) {
        throw "Run this script from the project's scripts folder, or from an extracted release zip."
    }
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw "cargo not found. Install Rust from https://rustup.rs (with the MSVC build tools) and try again."
    }
    Push-Location $Root
    try {
        cargo build --release
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    } finally {
        Pop-Location
    }
    $Built = Join-Path $Root "target\release\$ExeName"
    if (-not $Install) {
        Write-Host "Built $Built"
        exit 0
    }
}
if (-not $Install) {
    Write-Host "Prebuilt $Built found; use -Install to install it."
    exit 0
}

Stop-App
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$Exe = Join-Path $InstallDir $ExeName
Copy-Item $Built $Exe -Force
# MinGW (cross-compiled) builds load WebView2Loader.dll at runtime; MSVC builds link it statically
$Loader = Join-Path (Split-Path $Built -Parent) 'WebView2Loader.dll'
if (Test-Path $Loader) { Copy-Item $Loader (Join-Path $InstallDir 'WebView2Loader.dll') -Force }
Copy-Item $PSCommandPath (Join-Path $InstallDir 'uninstall.ps1') -Force

$shell = New-Object -ComObject WScript.Shell
$lnk = $shell.CreateShortcut($Shortcut)
$lnk.TargetPath       = $Exe
$lnk.WorkingDirectory = $InstallDir
$lnk.IconLocation     = "$Exe,0"
$lnk.Description      = 'Nonograms Katana user puzzles on the desktop'
$lnk.Save()

$version = (Get-Item $Exe).VersionInfo.ProductVersion
$sizeKb = [int]((Get-Item $Exe).Length / 1KB)
New-Item -Path $UninstallKey -Force | Out-Null
$props = @{
    DisplayName     = $AppName
    DisplayVersion  = $version
    DisplayIcon     = "$Exe,0"
    Publisher       = $AppName
    InstallLocation = $InstallDir
    UninstallString = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$InstallDir\uninstall.ps1`" -Uninstall"
}
foreach ($k in $props.Keys) { New-ItemProperty -Path $UninstallKey -Name $k -Value $props[$k] -PropertyType String -Force | Out-Null }
foreach ($k in 'NoModify', 'NoRepair') { New-ItemProperty -Path $UninstallKey -Name $k -Value 1 -PropertyType DWord -Force | Out-Null }
New-ItemProperty -Path $UninstallKey -Name 'EstimatedSize' -Value $sizeKb -PropertyType DWord -Force | Out-Null

Write-Host "Installed $AppName to $InstallDir (Start menu: $AppName)"
