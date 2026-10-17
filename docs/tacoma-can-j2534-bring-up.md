# 2017 Toyota Tacoma — J2534 bench bring-up (Rust Wideband)

Use this checklist with a **2017 Tacoma** (Denso ECM, **ISO 15765 / CAN 500 kbit/s**, 11-bit IDs `0x7E0` / `0x7E8`). Profile in the app: **Toyota Tacoma 2009+ (Denso CAN)** (`tacoma-can`).

Supported consumer cables (Windows, 32-bit PassThru DLL):

| Cable | Typical registry / dropdown name | Driver install |
| --- | --- | --- |
| **Mini-VCI** (MVCI / XHorse) | `MVCI`, `MiniVCI`, `PassThruPro`, etc. | Install vendor J2534 driver; DLL often under `C:\Program Files (x86)\MVCI\...` |
| **Mini-VCI clone** ("for TIS Techstream", XHorse driver) | `XHorse - MVCI` → shown as `[Mini-VCI J2534]` | Install the bundled "MVCI Driver for TOYOTA" package; DLL usually `C:\Program Files (x86)\XHorse Electronics\MVCI Driver for TOYOTA TIS\MVCI32.dll` |
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
6. **Identify ECU** — Real VIN / Cal IDs / CVNs from the bus (OBD Mode 09 first, UDS identifiers as fallback); empty bus **fails** (no demo VIN on real hardware). Fields the ECU does not answer are listed in the log instead of failing the whole read.
7. **Flash** — Only after `security-config.json` has your allowed masks/checksums configured; brick risk.

## How Wideband talks to the cable on CAN

On CAN profiles Wideband opens the cable in **ISO 15765** mode (J2534 protocol 6) with a flow-control filter, the same way Techstream does: the cable segments and reassembles diagnostic messages and answers flow control itself. Clone Mini-VCI cables are tested for this mode by their makers; several do not answer reliably in raw CAN mode. The Connect log line ends with `ISO 15765 (cable ISO-TP)` when this mode is active. If a cable refuses it, Wideband falls back to raw CAN (protocol 5) with its own ISO-TP and the log says `raw CAN (cable refused ISO 15765: …)`. Set the environment variable `WIDEBAND_RAW_CAN=1` before starting Wideband to force raw CAN.

## Sharing the cable with Techstream

Techstream and Wideband use the same J2534 driver, but only one program can hold the cable open. Click **Disconnect** (or **Release cable for Techstream**) before opening Techstream, and close Techstream before **Connect** in Wideband. If the cable is still held, Connect reports *Another program has this cable open*.

## Toyota Update Assistant (Flash & Diag tab)

Toyota calibration updates are installed by Toyota's software (Techstream with a TIS subscription). Wideband does not install them; the assistant covers the steps around the update and never writes to the ECU.

1. **Record ECU (before)** — reads VIN, every Cal ID (Mode 09 04; Toyota ECMs often report engine and transmission IDs), CVNs, and battery voltage. Saved to `wideband-ecu-before-update.json`; every record is also appended to `wideband-ecu-update-history.jsonl`. Both contain the VIN and are git-ignored.
2. **Check update file (.cuw)** — searches the file's text header for your current Cal ID and reports whether the payload is plain S-records or binary/encrypted. The `.cuw` header layout is not published, so this is a text match: confirm against the Toyota bulletin.
3. **Release cable for Techstream** — disconnects so Techstream can open the cable. Hold the battery at about 13.5 V with a charger in power-supply mode during the update.
4. **Verify ECU (after)** — reconnect, enter the bulletin's new Cal ID (optional), and compare against the before record.

Mini-VCI clone cables are fine for diagnostics but are a known risk for reflashing; the assistant flags them. Use a genuine J2534 interface for the update itself.

## Virtual ECU (bench only)

Choose **Wideband Virtual ECU Simulator** explicitly for offline protocol tests and `cargo test`. Demo identification defaults apply **only** on that device.

## Related

- [hardware-verification.md](https://github.com/ryanpeh139/Wideband/blob/main/docs/hardware-verification.md) — full H0–H9 checklist (repo copy when present)
- `README.md` — build `J2534Host.exe` with `/platform:x86`
