# Install CuqueClicker on Windows.
# Usage (PowerShell): irm https://raw.githubusercontent.com/flipbit03/cuqueclicker/main/install.ps1 | iex

$ErrorActionPreference = 'Stop'

$Repo = 'flipbit03/cuqueclicker'
$InstallDir = if ($env:CUQUECLICKER_INSTALL_DIR) {
    $env:CUQUECLICKER_INSTALL_DIR
} else {
    Join-Path $env:LOCALAPPDATA 'Programs\cuqueclicker'
}

# Architecture check — only x86_64 binaries are published.
$Arch = $env:PROCESSOR_ARCHITECTURE
if ($Arch -ne 'AMD64') {
    Write-Host "Unsupported architecture: $Arch. Only x86_64 (AMD64) binaries are published." -ForegroundColor Red
    Write-Host "Alternative: cargo install cuqueclicker"
    exit 1
}

$Asset = 'cuqueclicker_windows_x86_64.exe'

# Fetch latest release.
Write-Host "Fetching latest release..."
try {
    $Release = Invoke-RestMethod "https://api.github.com/repos/$Repo/releases/latest"
    $Tag = $Release.tag_name
} catch {
    Write-Host "Failed to fetch latest release: $_" -ForegroundColor Red
    exit 1
}
Write-Host "Latest release: $Tag"

# Download.
$Url = "https://github.com/$Repo/releases/download/$Tag/$Asset"
Write-Host "Downloading $Asset..."
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$BinaryPath = Join-Path $InstallDir 'cuqueclicker.exe'
$TempPath = "$BinaryPath.new"
$OldPath = "$BinaryPath.old"

# Clean up any .old left behind by a previous update (may still be locked if
# its process is somehow alive; silent failure is fine — we'll try again next run).
if (Test-Path $OldPath) {
    Remove-Item $OldPath -Force -ErrorAction SilentlyContinue
}

try {
    Invoke-WebRequest -Uri $Url -OutFile $TempPath -UseBasicParsing
} catch {
    Write-Host "Failed to download: $_" -ForegroundColor Red
    exit 1
}

# Windows locks running .exe files against overwrite — but it *does* allow
# renaming them. So when the target already exists (a previous install, and
# possibly the exact process that invoked `cuqueclicker self update`), move
# it aside first, then put the new binary in its place. The still-running
# old process keeps executing from the renamed `.old` path.
if (Test-Path $BinaryPath) {
    try {
        Move-Item -Path $BinaryPath -Destination $OldPath -Force
    } catch {
        Write-Host "Failed to move aside the existing binary: $_" -ForegroundColor Red
        Write-Host "If another CuqueClicker process is running, quit it and retry." -ForegroundColor Yellow
        Remove-Item $TempPath -Force -ErrorAction SilentlyContinue
        exit 1
    }
}
Move-Item -Path $TempPath -Destination $BinaryPath -Force

Write-Host ""
Write-Host "Installed cuqueclicker $Tag to $BinaryPath" -ForegroundColor Green

# Add to User PATH if not already present (case-insensitive check).
$UserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (-not $UserPath) { $UserPath = '' }
$AlreadyInPath = $false
foreach ($p in $UserPath.Split(';')) {
    if ($p -and ($p.TrimEnd('\') -ieq $InstallDir.TrimEnd('\'))) {
        $AlreadyInPath = $true
        break
    }
}
if (-not $AlreadyInPath) {
    $NewPath = if ([string]::IsNullOrEmpty($UserPath)) {
        $InstallDir
    } elseif ($UserPath.EndsWith(';')) {
        "$UserPath$InstallDir"
    } else {
        "$UserPath;$InstallDir"
    }
    [Environment]::SetEnvironmentVariable('Path', $NewPath, 'User')
    Write-Host "Added $InstallDir to your User PATH." -ForegroundColor Green
    Write-Host "Open a NEW terminal to pick up the change, then run: cuqueclicker"
} else {
    Write-Host "Run: cuqueclicker"
}

Write-Host ""
Write-Host "Note: The binary is not code-signed. On first launch, Windows Defender SmartScreen" -ForegroundColor DarkGray
Write-Host "may warn about an unrecognized app. Click 'More info' then 'Run anyway' to proceed." -ForegroundColor DarkGray
