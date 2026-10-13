<#
.SYNOPSIS
    Builds a release binary of Openbar5000 for Windows, and -- if the WiX
    Toolset CLI is available -- packages it into an MSI installer.
.DESCRIPTION
    Produces:
      dist\windows\openbar5000.exe                 (raw binary)
      dist\windows\Openbar5000-Setup-<version>.msi  (MSI installer, if WiX is installed)
.EXAMPLE
    .\compile\build-windows.ps1
#>

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = Split-Path -Parent $ScriptDir
$DistDir = Join-Path $RepoRoot "dist\windows"

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Error "cargo not found. Install Rust via https://rustup.rs first."
    exit 1
}

Write-Host "==> Building Openbar5000 (release) for Windows..."
Push-Location $RepoRoot
try {
    cargo build --release -p app
}
finally {
    Pop-Location
}

New-Item -ItemType Directory -Force -Path $DistDir | Out-Null
Copy-Item (Join-Path $RepoRoot "target\release\openbar5000.exe") (Join-Path $DistDir "openbar5000.exe") -Force
Write-Host "==> Binary: $DistDir\openbar5000.exe"

if (-not (Get-Command wix -ErrorAction SilentlyContinue)) {
    Write-Warning "Skipping MSI packaging: the WiX Toolset CLI ('wix') was not found. Install it with:`n    dotnet tool install --global wix`n(requires the .NET SDK: https://dotnet.microsoft.com/download) and re-run this script."
    exit 0
}

# The UI extension (WixUI dialogs) isn't installed by default -- add it if
# missing rather than failing the build over a one-time setup step.
$extensions = & wix extension list --global 2>$null
if (-not ($extensions -match "WixToolset\.UI\.wixext")) {
    Write-Host "==> Installing WiX UI extension..."
    & wix extension add WixToolset.UI.wixext --global
    if ($LASTEXITCODE -ne 0) {
        Write-Error "Failed to install the WiX UI extension (exit code $LASTEXITCODE)."
        exit 1
    }
}

$CargoToml = Get-Content (Join-Path $RepoRoot "Cargo.toml") -Raw
$Version = "0.0.0"
if ($CargoToml -match '(?m)^version\s*=\s*"([^"]+)"') {
    $Version = $Matches[1]
}

$MsiPath = Join-Path $DistDir "Openbar5000-Setup-$Version.msi"
Write-Host "==> Packaging MSI with WiX (version $Version)..."
& wix build (Join-Path $ScriptDir "installer.wxs") `
    -d "Version=$Version" `
    -d "RepoRoot=$RepoRoot" `
    -ext WixToolset.UI.wixext `
    -arch x64 `
    -o $MsiPath
if ($LASTEXITCODE -ne 0) {
    Write-Error "WiX build failed (exit code $LASTEXITCODE)."
    exit 1
}

Write-Host "==> Done: $MsiPath"
