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

On CAN profiles Wideband opens the cable in **raw CAN** mode (J2534 protocol 5) and runs ISO-TP itself: segmentation, reassembly, and flow control. Raw CAN reads honor their timeout, so a quiet bus never stalls the program.

The cable's own **ISO 15765** mode (J2534 protocol 6, with a flow-control filter) is opt-in: start Wideband with `--cable-isotp` or set `WIDEBAND_CABLE_ISOTP=1`. The Connect log line then ends with `ISO 15765 (cable ISO-TP)`. On the XHorse Mini-VCI driver (`MVCI32.dll` 1.4.3, firmware `J2534 MINIV1.03`), every read on an ISO 15765 channel with nothing to receive blocks about 5 s, whatever timeout it is given. Calling the driver from a second thread while that read is pending makes writes hang, so this mode stays off by default. `--raw-can` / `WIDEBAND_RAW_CAN=1` still force raw CAN.

**Identify ECU** sends the OBD Mode 09 requests (VIN `09 02`, Cal ID `09 04`, CVN `09 06`) to the functional id `0x7DF`, as scan tools do. Replies are read from `0x7E8`, and flow control goes to `0x7E0`. On the bench 2017 Tacoma (ignition ON), the ECM answered these requests on `0x7DF` and accepted flow control on `0x7E0`. It did not answer any single-frame request sent physically to `0x7E0` (`01 00`, `09 02`, `3E 00`, `10 81`, `22 11 90`, `1A 90`, with `AA`, `00`, or `55` padding). Services that must go to the physical id therefore still need investigation on this truck.

Each cable session writes `j2534-trace.log` beside `wideband.exe`: every command sent to `J2534Host.exe`, its reply, and how long it took. If Wideband stops responding with a cable connected, the last lines show which call the cable driver did not return from.

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
