<#
.SYNOPSIS
  Builds dist\Wideband-Setup.exe on Windows (the Windows counterpart of build-windows-installer.sh).

.DESCRIPTION
  Builds wideband.exe (release), compiles J2534Host.exe with the 32-bit csc.exe and checks it with
  --check-pack, stages the same files as the Linux script, and runs makensis on
  installer\wideband.nsi. The installer keeps an existing security-config.json.

.PARAMETER SkipCargo
  Use the existing target\release\wideband.exe.

.EXAMPLE
  .\scripts\Build-WindowsInstaller.ps1
#>
param(
    [switch]$SkipCargo
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$stage = Join-Path $repo "dist\installer-stage"
$out = Join-Path $repo "dist\Wideband-Setup.exe"

function Write-Step($msg) {
    Write-Host ("==> " + $msg) -ForegroundColor Cyan
}

$makensis = @(
    (Get-Command makensis -ErrorAction SilentlyContinue | ForEach-Object Source),
    "${env:ProgramFiles(x86)}\NSIS\makensis.exe",
    "$env:ProgramFiles\NSIS\makensis.exe"
) | Where-Object { $_ -and (Test-Path -LiteralPath $_) } | Select-Object -First 1
if (-not $makensis) {
    throw "makensis.exe not found - install NSIS 3 (winget install NSIS.NSIS)."
}

$version = (Select-String -Path (Join-Path $repo "Cargo.toml") -Pattern '^version = "(.*)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$commit = (& git -C $repo rev-parse --short HEAD 2>$null)
if (-not $commit) { $commit = "local" }

if (-not $SkipCargo) {
    Write-Step "cargo build --release"
    Push-Location $repo
    try {
        # cargo reports progress on stderr; run it through cmd so PowerShell does not treat it as an error.
        cmd /c "cargo build --release --locked 2>&1" | Write-Host
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }
    } finally {
        Pop-Location
    }
}

Write-Step "stage $stage"
if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
New-Item -ItemType Directory -Force -Path $stage | Out-Null
Copy-Item -LiteralPath (Join-Path $repo "target\release\wideband.exe") -Destination $stage

$csc32 = Join-Path $env:WINDIR "Microsoft.NET\Framework\v4.0.30319\csc.exe"
& $csc32 /nologo /warnaserror /target:exe /platform:x86 "/out:$stage\J2534Host.exe" (Join-Path $repo "J2534Host.cs")
if ($LASTEXITCODE -ne 0) { throw "J2534Host compile failed" }
& "$stage\J2534Host.exe" --check-pack | Out-Null
if ($LASTEXITCODE -ne 0) { throw "J2534Host.exe --check-pack failed" }

foreach ($name in "security-config.json", "flash-layout.json", "telemetry-channels.json", "strategy-managers.json", "device_check.ps1") {
    Copy-Item -LiteralPath (Join-Path $repo $name) -Destination $stage
}
Copy-Item -LiteralPath (Join-Path $repo "scripts\Friday-CablePrep.ps1") -Destination $stage
Copy-Item -LiteralPath (Join-Path $repo "docs\tacoma-can-j2534-bring-up.md") -Destination (Join-Path $stage "Tacoma-Setup-Guide.md")
$readme = Get-Content -LiteralPath (Join-Path $repo "installer\README-FIRST.txt")
Set-Content -LiteralPath (Join-Path $stage "README-FIRST.txt") -Value $readme -Encoding ascii
Copy-Item -LiteralPath (Join-Path $repo "profiles") -Destination $stage -Recurse
Copy-Item -LiteralPath (Join-Path $repo "definition-packs") -Destination $stage -Recurse

Write-Step "makensis"
& $makensis -V2 "-DSTAGE=$stage" "-DVERSION=$version ($commit)" "-DOUTFILE=$out" (Join-Path $repo "installer\wideband.nsi")
if ($LASTEXITCODE -ne 0) { throw "makensis failed with exit code $LASTEXITCODE" }
Write-Host ""
Write-Host "Installer: $out" -ForegroundColor Green
