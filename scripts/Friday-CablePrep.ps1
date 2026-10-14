<#
.SYNOPSIS
  Cable-day prep: verify release folder + optional J2534 DLL probe before opening Wideband.

.DESCRIPTION
  Prints PASS/FAIL lines for Friday Tacoma bring-up. Required files beside wideband.exe:
  J2534Host.exe, security-config.json, flash-layout.json, telemetry-channels.json,
  profiles\tacoma-can.json. Optionally runs device_check.ps1 when -DllPath is set.

.PARAMETER ReleaseDir
  Folder containing wideband.exe (defaults to current directory).

.PARAMETER DllPath
  Full path to your PassThru FunctionLibrary DLL (Mini-VCI / VXDIAG). When set, runs device_check.ps1.

.EXAMPLE
  .\scripts\Friday-CablePrep.ps1 -ReleaseDir .\target\release -DllPath "C:\Program Files (x86)\...\mvci.dll"
#>
param(
    [string]$ReleaseDir = ".",
    [string]$DllPath = ""
)

$ErrorActionPreference = "Continue"
$ProgressPreference = "SilentlyContinue"

$ReleaseDir = (Resolve-Path -LiteralPath $ReleaseDir).Path
$script:failCount = 0
$script:passCount = 0

function Write-Result($name, $ok, $detail) {
    if ($ok) {
        $script:passCount++
        Write-Host ("PASS  " + $name + " — " + $detail) -ForegroundColor Green
    } else {
        $script:failCount++
        Write-Host ("FAIL  " + $name + " — " + $detail) -ForegroundColor Red
    }
}

function Test-FileBeside($label, $relative) {
    $path = Join-Path $ReleaseDir $relative
    $ok = Test-Path -LiteralPath $path
    $detail = if ($ok) { $path } else { "missing (expected $path)" }
    Write-Result $label $ok $detail
    return $ok
}

Write-Host ""
Write-Host "Wideband cable-day prep" -ForegroundColor Cyan
Write-Host "Release folder: $ReleaseDir"
Write-Host ""

Test-FileBeside "wideband.exe" "wideband.exe" | Out-Null
Test-FileBeside "J2534Host.exe (x86)" "J2534Host.exe" | Out-Null
Test-FileBeside "security-config.json" "security-config.json" | Out-Null
Test-FileBeside "flash-layout.json" "flash-layout.json" | Out-Null
Test-FileBeside "telemetry-channels.json" "telemetry-channels.json" | Out-Null
Test-FileBeside "profiles\tacoma-can.json" "profiles\tacoma-can.json" | Out-Null

$deviceCheck = Join-Path $ReleaseDir "device_check.ps1"
if (-not (Test-Path -LiteralPath $deviceCheck)) {
    $deviceCheck = Join-Path (Split-Path $PSScriptRoot -Parent) "device_check.ps1"
}
$hasDeviceCheck = Test-Path -LiteralPath $deviceCheck
Write-Result "device_check.ps1 present" $hasDeviceCheck $(if ($hasDeviceCheck) { $deviceCheck } else { "not found beside exe or repo root" })

# Registry: at least one PassThruSupport.04.04 entry with FunctionLibrary on disk
$regOk = $false
$regDetail = "no PassThruSupport.04.04 entries with an existing FunctionLibrary DLL"
$regRoots = @(
    "HKLM:\SOFTWARE\WOW6432Node\PassThruSupport.04.04",
    "HKLM:\SOFTWARE\PassThruSupport.04.04"
)
$found = @()
foreach ($root in $regRoots) {
    if (-not (Test-Path -LiteralPath $root)) { continue }
    Get-ChildItem -LiteralPath $root -ErrorAction SilentlyContinue | ForEach-Object {
        try {
            $lib = (Get-ItemProperty -LiteralPath $_.PSPath -Name FunctionLibrary -ErrorAction Stop).FunctionLibrary
            $name = (Get-ItemProperty -LiteralPath $_.PSPath -Name Name -ErrorAction SilentlyContinue).Name
            if (-not $name) { $name = $_.PSChildName }
            if ($lib -and (Test-Path -LiteralPath $lib)) {
                $found += "$name -> $lib"
            }
        } catch { }
    }
}
if ($found.Count -gt 0) {
    $regOk = $true
    $regDetail = ($found | Select-Object -First 3) -join "; "
    if ($found.Count -gt 3) { $regDetail += " (+$($found.Count - 3) more)" }
}
Write-Result "J2534 registry + DLL on disk" $regOk $regDetail

# security-config: tacoma-can profile row exists (configured may still be false — fail-closed is OK)
$secOk = $false
$secDetail = "could not read security-config.json"
$secPath = Join-Path $ReleaseDir "security-config.json"
if (Test-Path -LiteralPath $secPath) {
    try {
        $sec = Get-Content -LiteralPath $secPath -Raw | ConvertFrom-Json
        if ($sec.profiles -and $sec.profiles."tacoma-can") {
            $row = $sec.profiles."tacoma-can"
            $configured = $false
            if ($row.PSObject.Properties.Name -contains "configured") {
                $configured = [bool]$row.configured
            }
            $secOk = $true
            if ($configured) {
                $secDetail = "profiles.tacoma-can present, configured=true (production keys active)"
            } else {
                $secDetail = "profiles.tacoma-can present, configured=false (fail-closed flash — expected until you set OEM mask)"
            }
        } else {
            $secDetail = "profiles.tacoma-can missing in JSON"
        }
    } catch {
        $secDetail = "parse error: $($_.Exception.Message)"
    }
}
Write-Result "security-config tacoma-can row" $secOk $secDetail

if ([string]::IsNullOrWhiteSpace($DllPath)) {
    Write-Result "device_check.ps1 probe (optional)" $true "skipped — pass -DllPath to your PassThru FunctionLibrary"
} elseif (-not $hasDeviceCheck) {
    Write-Result "device_check.ps1 probe" $false "device_check.ps1 not found"
} else {
    try {
        $json = & powershell -NoProfile -ExecutionPolicy Bypass -File $deviceCheck -DllPath $DllPath
        $probe = $json | ConvertFrom-Json
        if ($probe.ok) {
            Write-Result "device_check.ps1 probe" $true ("firmware=" + $probe.firmware + " dll=" + $probe.dll)
        } else {
            Write-Result "device_check.ps1 probe" $false ($probe.error)
        }
    } catch {
        Write-Result "device_check.ps1 probe" $false $_.Exception.Message
    }
}

Write-Host ""
if ($script:failCount -eq 0) {
    Write-Host "OVERALL: PASS ($script:passCount checks)" -ForegroundColor Green
    Write-Host "You may open wideband.exe — use profile tacoma-can and your registry J2534 device (not Virtual ECU)."
    exit 0
} else {
    Write-Host "OVERALL: FAIL ($script:failCount failed, $script:passCount passed)" -ForegroundColor Red
    Write-Host "Fix FAIL lines above. Typical fixes:"
    Write-Host "  • Run scripts\Package-WindowsRelease.ps1 to copy J2534Host + profiles"
    Write-Host "  • Install Mini-VCI / VXDIAG J2534 driver (PassThruSupport registry)"
    Write-Host "  • device_check.ps1 -DllPath <your FunctionLibrary path>"
    exit 1
}
