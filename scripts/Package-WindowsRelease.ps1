<#
.SYNOPSIS
  Build wideband.exe (release) and J2534Host.exe (x86), then stage a cable-ready folder.

.DESCRIPTION
  Output folder contains wideband.exe, J2534Host.exe, JSON configs, and profiles/.
  Run scripts\Friday-CablePrep.ps1 against that folder before truck day.

.PARAMETER RepoRoot
  Wideband repository root (defaults to parent of this scripts folder).

.PARAMETER OutDir
  Staging directory (defaults to <RepoRoot>\target\release).

.PARAMETER SkipCargo
  Skip `cargo build --release` (only rebuild/copy host + configs).

.EXAMPLE
  .\scripts\Package-WindowsRelease.ps1
#>
param(
    [string]$RepoRoot = "",
    [string]$OutDir = "",
    [switch]$SkipCargo
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

if ([string]::IsNullOrWhiteSpace($RepoRoot)) {
    $RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
} else {
    $RepoRoot = (Resolve-Path $RepoRoot).Path
}

if ([string]::IsNullOrWhiteSpace($OutDir)) {
    $OutDir = Join-Path $RepoRoot "target\release"
} else {
    $OutDir = (Resolve-Path -LiteralPath $OutDir -ErrorAction SilentlyContinue)
    if (-not $OutDir) {
        New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
        $OutDir = (Resolve-Path $OutDir).Path
    }
}

function Write-Step($msg) {
    Write-Host ("==> " + $msg) -ForegroundColor Cyan
}

Write-Step "Repo: $RepoRoot"
Write-Step "Release folder: $OutDir"

if (-not $SkipCargo) {
    Write-Step "cargo build --release"
    Push-Location $RepoRoot
    try {
        cargo build --release
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }
    } finally {
        Pop-Location
    }
}

$widebandSrc = Join-Path $RepoRoot "target\release\wideband.exe"
if (-not (Test-Path -LiteralPath $widebandSrc)) {
    throw "wideband.exe not found at $widebandSrc — run without -SkipCargo or build manually."
}

Write-Step "Build J2534Host.exe (x86)"
$hostCs = Join-Path $RepoRoot "J2534Host.cs"
if (-not (Test-Path -LiteralPath $hostCs)) {
    throw "J2534Host.cs not found at $hostCs"
}

$csc32 = Join-Path $env:WINDIR "Microsoft.NET\Framework\v4.0.30319\csc.exe"
if (-not (Test-Path -LiteralPath $csc32)) {
    throw "32-bit csc.exe not found at $csc32 — install .NET Framework 4.x developer pack."
}

$hostOut = Join-Path $OutDir "J2534Host.exe"
& $csc32 /nologo /warnaserror /target:exe /platform:x86 /out:$hostOut $hostCs
if ($LASTEXITCODE -ne 0) { throw "J2534Host compile failed" }

Write-Step "Verify J2534Host packing"
& $hostOut --check-pack
if ($LASTEXITCODE -ne 0) { throw "J2534Host.exe --check-pack failed" }

Write-Step "Copy configs and profiles"
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

Copy-Item -LiteralPath $widebandSrc -Destination (Join-Path $OutDir "wideband.exe") -Force

$configFiles = @(
    "security-config.json",
    "flash-layout.json",
    "telemetry-channels.json",
    "device_check.ps1"
)
foreach ($name in $configFiles) {
    $src = Join-Path $RepoRoot $name
    if (-not (Test-Path -LiteralPath $src)) {
        throw "Missing required file: $src"
    }
    Copy-Item -LiteralPath $src -Destination (Join-Path $OutDir $name) -Force
}

$profilesSrc = Join-Path $RepoRoot "profiles"
$profilesDst = Join-Path $OutDir "profiles"
if (Test-Path -LiteralPath $profilesSrc) {
    if (Test-Path -LiteralPath $profilesDst) {
        Remove-Item -LiteralPath $profilesDst -Recurse -Force
    }
    Copy-Item -LiteralPath $profilesSrc -Destination $profilesDst -Recurse -Force
} else {
    throw "profiles\ folder missing at $profilesSrc"
}

Write-Host ""
Write-Host "Windows release staged at:" -ForegroundColor Green
Write-Host "  $OutDir"
Write-Host ""
Write-Host "Next: cd to that folder and run:" -ForegroundColor Yellow
Write-Host "  .\..\..\scripts\Friday-CablePrep.ps1 -ReleaseDir `"$OutDir`""
Write-Host "  (or copy Friday-CablePrep.ps1 beside wideband.exe and run it there)"
