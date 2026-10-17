# Windows release folder (cable-ready)

Ship **one directory** beside the truck laptop. The Rust app resolves configs and `profiles/` from the folder that contains `wideband.exe`.

## Quick path (PowerShell)

From the repo root on Windows:

```powershell
.\scripts\Package-WindowsRelease.ps1
.\scripts\Friday-CablePrep.ps1 -ReleaseDir .\dist\windows-release -DllPath "C:\path\to\your\PassThru.dll"
```

`Package-WindowsRelease.ps1`:

1. `cargo build --release` (binary lands in `target\release\`)
2. Compiles `J2534Host.exe` with `csc.exe /platform:x86`
3. Runs `J2534Host.exe --check-pack`
4. Stages `wideband.exe`, JSON configs, `device_check.ps1`, and the full `profiles\` tree into `dist\windows-release\` (distinct from cargo's build output; same-path copies are skipped)

`Friday-CablePrep.ps1` prints **PASS/FAIL** for each Friday blocker (host, configs, `profiles\tacoma-can.json`, registry J2534, optional DLL probe).

## Manual copy list

If you cannot run the script, copy into the same folder as `wideband.exe`:

| Item | Source |
| --- | --- |
| `wideband.exe` | `cargo build --release` |
| `J2534Host.exe` | `csc.exe /platform:x86 /out:J2534Host.exe J2534Host.cs` |
| `security-config.json` | repo root (or copy from `tests/security-config-tacoma-can.sample.json`) |
| `flash-layout.json` | repo root |
| `telemetry-channels.json` | repo root |
| `profiles\` | repo `profiles\` (at least `tacoma-can.json`) |
| `device_check.ps1` | repo root (recommended before connect) |

## Installer (Wideband-Setup.exe)

On Windows, with NSIS 3 installed (`winget install NSIS.NSIS`):

```powershell
.\scripts\Build-WindowsInstaller.ps1
```

It builds the release, compiles and checks `J2534Host.exe`, stages the same files as `scripts/build-windows-installer.sh`, and writes `dist\Wideband-Setup.exe`. The installer installs per user to `%LOCALAPPDATA%\Programs\Wideband` with Start menu and desktop shortcuts, updates an existing install in place, and keeps an existing `security-config.json`.

## Related

- [tacoma-can-j2534-bring-up.md](./tacoma-can-j2534-bring-up.md)
- Motorsport Studio Project store: `reality-check.md`, `mini-vci-tacoma-bringup.md`
