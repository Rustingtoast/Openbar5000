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

# The license agreement the installer shows, as RTF (all WiX's license page
# takes): the terms (compile\installer-terms.txt), then the Apache license
# itself (LICENSE). Both are plain text: paragraphs are separated by blank
# lines and rewrapped to fit the dialog, "# " starts a heading and "- " a
# bullet point.
function ConvertTo-Rtf([string[]]$Lines) {
    function Escape([string]$Text) {
        $out = New-Object System.Text.StringBuilder
        foreach ($c in $Text.ToCharArray()) {
            $code = [int]$c
            if ($c -eq '\' -or $c -eq '{' -or $c -eq '}') { [void]$out.Append('\').Append($c) }
            elseif ($code -lt 128) { [void]$out.Append($c) }
            else {
                # RTF's \u takes a signed 16-bit value, then a fallback character.
                if ($code -gt 32767) { $code -= 65536 }
                [void]$out.Append("\u$code?")
            }
        }
        $out.ToString()
    }

    $blocks = New-Object System.Collections.Generic.List[string]
    $block = New-Object System.Collections.Generic.List[string]
    foreach ($line in ($Lines + '')) {
        $text = $line.Trim()
        if ($text -eq '' -or $text.StartsWith('# ') -or $text.StartsWith('- ')) {
            if ($block.Count) { $blocks.Add($block -join ' '); $block.Clear() }
        }
        if ($text -ne '') { $block.Add($text) }
        if ($text.StartsWith('# ')) { $blocks.Add($text); $block.Clear() }
    }

    $body = foreach ($text in $blocks) {
        if ($text.StartsWith('# ')) { '\pard\sb200\sa100\b\fs20 ' + (Escape $text.Substring(2)) + '\b0\fs18\par' }
        elseif ($text.StartsWith('- ')) { '\pard\li360\fi-240\tx360\sa100 \bullet\tab ' + (Escape $text.Substring(2)) + '\par' }
        else { '\pard\sa120 ' + (Escape $text) + '\par' }
    }
    "{\rtf1\ansi\ansicpg1252\deff0\uc1{\fonttbl{\f0\fswiss\fcharset0 Segoe UI;}}\f0\fs18`r`n" + ($body -join "`r`n") + "`r`n}"
}

$WixDir = Join-Path $RepoRoot "target\wix"
New-Item -ItemType Directory -Force -Path $WixDir | Out-Null
$LicenseRtf = Join-Path $WixDir "License.rtf"
$Terms = Get-Content (Join-Path $ScriptDir "installer-terms.txt") -Encoding UTF8
$License = Get-Content (Join-Path $RepoRoot "LICENSE") -Encoding UTF8
[System.IO.File]::WriteAllText($LicenseRtf, (ConvertTo-Rtf ($Terms + '' + '# Apache License, Version 2.0' + '' + $License)), [System.Text.Encoding]::ASCII)

# An MSI version is numbers only: a pre-release ("0.7.1-alpha") installs as
# its release number, and is named in the file.
$MsiVersion = ($Version -split '-', 2)[0]

$MsiPath = Join-Path $DistDir "Openbar5000-Setup-$Version.msi"
Write-Host "==> Packaging MSI with WiX (version $Version)..."
& wix build (Join-Path $ScriptDir "installer.wxs") `
    -d "Version=$MsiVersion" `
    -d "RepoRoot=$RepoRoot" `
    -d "LicenseRtf=$LicenseRtf" `
    -ext WixToolset.UI.wixext `
    -arch x64 `
    -o $MsiPath
if ($LASTEXITCODE -ne 0) {
    Write-Error "WiX build failed (exit code $LASTEXITCODE)."
    exit 1
}

Write-Host "==> Done: $MsiPath"
