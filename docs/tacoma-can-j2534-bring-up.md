# 2017 Toyota Tacoma — J2534 bench bring-up (Rust Wideband)

Use this checklist with a **2017 Tacoma** (Denso ECM, **ISO 15765 / CAN 500 kbit/s**, 11-bit IDs `0x7E0` / `0x7E8`). Profile in the app: **Toyota Tacoma 2009+ (Denso CAN)** (`tacoma-can`).

Supported consumer cables (Windows, 32-bit PassThru DLL):

| Cable | Typical registry / dropdown name | Driver install |
| --- | --- | --- |
| **Mini-VCI** (MVCI / XHorse) | `MVCI`, `MiniVCI`, `PassThruPro`, etc. | Install vendor J2534 driver; DLL often under `C:\Program Files (x86)\MVCI\...` |
| **VXDIAG VCX Nano (Toyota)** | `VXDIAG`, `VCX`, `Toyota J2534`, etc. | Install **VX Manager** and enable **Toyota J2534**; DLL often under `C:\Program Files (x86)\VXDIAG\VcxNv_Toyota\` |

Wideband discovers devices from `PassThruSupport.04.04` registry keys and common DLL paths. Place **`J2534Host.exe`** (32-bit, built from `J2534Host.cs`) beside **`wideband.exe`** — the Rust app talks to the cable through that host on 64-bit Windows.

**Stage a cable-ready folder:** `.\scripts\Package-WindowsRelease.ps1` then `.\scripts\Friday-CablePrep.ps1 -ReleaseDir .\dist\windows-release -DllPath "<your PassThru DLL>"` — see [windows-release-packaging.md](./windows-release-packaging.md).

## Prerequisites

- Windows 10/11 x64, `cargo build --release`
- Ignition **ON** (engine off is fine for first tests), OBD-II connected
- Copy `security-config.json` next to `wideband.exe` — leave keys/checksums **unconfigured** until you intentionally flash
- Optional: `device_check.ps1 -DllPath "C:\path\to\your.dll"` to prove the DLL opens outside Wideband

## Test order (real vehicle — no flash until configured)

1. **Device** — Select your **Mini-VCI** or **VXDIAG** entry (not *Virtual ECU* unless bench testing).
2. **Profile** — **Toyota Tacoma 2009+ (Denso CAN)**.
3. **Connect** — Log should show PassThru channel open, filter on `0x7E8`, and firmware/DLL strings from `J2534Host` VERSION when available.
4. **Telemetry** — RPM / MAP / etc. should update with engine running; header **VBATT** shows `—` until ECU or `READ_VBATT` provides a value (not a fake 13.4 V).
5. **Flash & Diag → Read DTCs** — Read-only stored codes / readiness (no clear without product policy).
6. **Identify ECU** — Real VIN / Cal / CVN from the bus; empty bus **fails** (no demo VIN on real hardware).
7. **Flash** — Only after `security-config.json` has your allowed masks/checksums configured; brick risk.

## Virtual ECU (bench only)

Choose **Wideband Virtual ECU Simulator** explicitly for offline protocol tests and `cargo test`. Demo identification defaults apply **only** on that device.

## Related

- [hardware-verification.md](https://github.com/ryanpeh139/Wideband/blob/main/docs/hardware-verification.md) — full H0–H9 checklist (repo copy when present)
- `README.md` — build `J2534Host.exe` with `/platform:x86`
