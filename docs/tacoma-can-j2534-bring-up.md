# 2017 Toyota Tacoma — J2534 bench bring-up (Rust Wideband)

Use this checklist with a **2017 Tacoma** (Denso ECM, **ISO 15765 / CAN 500 kbit/s**, 11-bit IDs: diagnostics `0x700` / `0x708`, OBD-II `0x7DF` → `0x7E8`). Profile in the app: **Toyota Tacoma 2009+ (Denso CAN)** (`tacoma-can`).

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
3. **Connect** — Log should show PassThru channel open, RX filter on `0x708` (the acceptance mask also passes `0x7E8`), and firmware/DLL strings from `J2534Host` VERSION when available.
4. **Telemetry** — RPM / MAP / etc. should update with engine running; header **VBATT** shows `—` until ECU or `READ_VBATT` provides a value (not a fake 13.4 V).
5. **Flash & Diag → Read DTCs** — Read-only stored codes / readiness (no clear without product policy).
6. **Identify ECU** — Real VIN / Cal IDs / CVNs from the bus (OBD Mode 09 first, UDS identifiers as fallback); empty bus **fails** (no demo VIN on real hardware). Fields the ECU does not answer are listed in the log instead of failing the whole read.
7. **Flash** — Only after `security-config.json` has your allowed masks/checksums configured; brick risk.

## How Wideband talks to the cable on CAN

On CAN profiles Wideband opens the cable in **raw CAN** mode (J2534 protocol 5) and runs ISO-TP itself: segmentation, reassembly, and flow control. Raw CAN reads honor their timeout, so a quiet bus never stalls the program.

The cable's own **ISO 15765** mode (J2534 protocol 6, with a flow-control filter) is opt-in: start Wideband with `--cable-isotp` or set `WIDEBAND_CABLE_ISOTP=1`. The Connect log line then ends with `ISO 15765 (cable ISO-TP)`. On the XHorse Mini-VCI driver (`MVCI32.dll` 1.4.3, firmware `J2534 MINIV1.03`), every read on an ISO 15765 channel with nothing to receive blocks about 5 s, whatever timeout it is given. Calling the driver from a second thread while that read is pending makes writes hang, so this mode stays off by default. `--raw-can` / `WIDEBAND_RAW_CAN=1` still force raw CAN.

### ECM addressing on the 2017 Tacoma (2GR-FKS)

The ECM uses two id pairs, and the `tacoma-can` profile uses both:

| Traffic | Request | Reply | Flow control from Wideband |
| --- | --- | --- | --- |
| UDS diagnostics: sessions `10`, TesterPresent `3E`, DIDs `22`, DTCs `19`, security, flash | `0x700` | `0x708` | `0x700` |
| OBD-II: Mode 01 live data, Mode 09 VIN / Cal ID / CVN | `0x7DF` (functional) | `0x7E8` | `0x7E0` |

The ECM answers no single-frame request sent physically to `0x7E0`. That includes `01 00`, `09 02`, `3E 00`, `10 81`, `22 11 90`, and `1A 90`, with `AA`, `00`, or `55` padding. It only accepts flow control there during an OBD reply. Techstream (17.30) gets no answer on `0x7E0` either. A logged Health Check shows it reaching the ECM on `0x700` → `0x708`: `10 01` → `50 01 00 32 01 F4`, `10 03`, `22 10 00`, `22 1F 00`, `22 01 0x`, `19 02`/`19 05`. It sends OBD requests to `0x7DF`.

In profile packs, `can.request_id` / `can.response_id` are the diagnostic pair, and the optional `can.obd_request_id` / `can.obd_response_id` give the OBD pair (default: the diagnostic pair). Live data routes by service: Mode 01 goes to `0x7DF` and `0x22` DIDs go to `0x700`. Raw CAN has one acceptance filter, so its mask passes both `0x708` and `0x7E8`.

Read-only bench check with the cable (ignition ON):

```
$env:WIDEBAND_HW_DLL = "C:\Program Files (x86)\Autokent MVCI MultiDriver X64\1.4.3\MVCI32.dll"
cargo test --release hardware_ -- --ignored --nocapture --test-threads=1
```

### Logging another J2534 program (`tools/j2534-sniff`)

`tools/j2534-sniff` builds a 32-bit J2534 DLL that forwards every PassThru call unchanged to the real driver. It logs the arguments, filters, IOCTLs, and every frame to `j2534-sniff.log` beside the DLL. Build it with `cargo build --release` in that folder; this needs the `i686-pc-windows-gnu` target and `i686-w64-mingw32-gcc` on `PATH`. Copy `target\i686-pc-windows-gnu\release\j2534_sniff.dll` to `tools\j2534-sniff\bin\`. Then import `add-sniff-vim.reg` as administrator; it adds a separate **Wideband Sniff - MVCI 1.4.3** device and leaves the XHorse entry alone. Select that device in Techstream's VIM Select. `remove-sniff-vim.reg` removes it again. The forwarded DLL is the XHorse `MVCI32.dll` unless `real-dll.txt` beside the logger names another.

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
