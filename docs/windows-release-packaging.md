# Windows release folder (cable-ready)

Ship **one directory** beside the truck laptop. The Rust app resolves configs and `profiles/` from the folder that contains `wideband.exe`.

## Quick path (PowerShell)

From the repo root on Windows:

```powershell
.\scripts\Package-WindowsRelease.ps1
.\scripts\Friday-CablePrep.ps1 -ReleaseDir .\target\release -DllPath "C:\path\to\your\PassThru.dll"
```

`Package-WindowsRelease.ps1`:

1. `cargo build --release`
2. Compiles `J2534Host.exe` with `csc.exe /platform:x86`
3. Runs `J2534Host.exe --check-pack`
4. Copies `wideband.exe`, JSON configs, `device_check.ps1`, and the full `profiles\` tree into `target\release\`

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

## Related

- [tacoma-can-j2534-bring-up.md](./tacoma-can-j2534-bring-up.md)
- Motorsport Studio Project store: `reality-check.md`, `mini-vci-tacoma-bringup.md`
