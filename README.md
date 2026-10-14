# Wideband

This folder is the Wideband project. Open the folder in another editor, or clone the private repository https://github.com/ryanpeh139/Wideband branch `main`. The GitHub account is `ryanpeh139`. Push later work to that remote. Do not force-push. Do not commit `XAI_API_KEY` or any other secret.

## Rust Motorsport Studio (primary UI)

The active desktop UI is the native Rust **egui** app under `src/`. Build and test from the repo root (Rust **1.88+**, see `rust-toolchain.toml`):

```
cargo build
cargo test
```

Run the binary as `wideband` (or `target/debug/wideband` / `target/release/wideband`). J2534 bench logic, XDF parsing, ISO-TP/UDS flash paths, and the motorsport calibration workspace live in `src/`.

**ECU profiles:** JSON packs under [`profiles/`](profiles/) (see [`docs/profile-packs.md`](docs/profile-packs.md)). **TCM** stubs live in [`profiles/tcm/`](profiles/tcm/) ([`docs/tcm-profile-packs.md`](docs/tcm-profile-packs.md)). Ship `profiles/` beside the binary on Windows; keys and checksum ranges remain in `security-config.json` only.

**Community definitions & plugins:** calibration packs under [`definition-packs/`](definition-packs/), native [`plugins/`](plugins/) (`cdylib`), and optional user paths — see [`docs/community-packs-and-plugins.md`](docs/community-packs-and-plugins.md).

**2017 Tacoma + Mini-VCI or VXDIAG VCX Nano (Toyota):** see [`docs/tacoma-can-j2534-bring-up.md`](docs/tacoma-can-j2534-bring-up.md). Place **`J2534Host.exe`** (32-bit, from `J2534Host.cs`) next to **`wideband.exe`** on Windows.

The legacy C# WinForms tree (`Wideband.cs`, `Calibrate.cs`, …) remains for reference and Windows-only `csc.exe` handoff; new UI work targets Rust.

## Handoff

The compiler is C# 5 on .NET Framework 4, through `csc.exe`. Keep the source on that language. String interpolation, `?.`, `nameof`, `out var`, expression-bodied members, and discards fail the build. PowerShell 5.1 has no `&&`. Chain commands with `;`.

`Wideband.exe` is the 64-bit WinForms program. `J2534Host.exe` is a separate 32-bit console program beside it, and it is the only process that loads `esp32.dll`. Leave `J2534Host.cs` out of the winexe compile. The namespace and window title are `Wideband`. `J2534Host.cs` has no namespace.

Build the app from this folder with every `.cs` file except `J2534Host.cs`:

```
C:\Windows\Microsoft.NET\Framework64\v4.0.30319\csc.exe /nologo /warnaserror /target:winexe /platform:anycpu /out:Wideband.exe /reference:System.Windows.Forms.dll /reference:System.Drawing.dll /reference:System.Web.Extensions.dll /reference:System.Xml.dll
```

Build the cable host:

```
C:\Windows\Microsoft.NET\Framework\v4.0.30319\csc.exe /nologo /warnaserror /target:exe /platform:x86 /out:J2534Host.exe J2534Host.cs
```

`/warnaserror` is on. A public field that other code assigns needs an initializer. In WinForms, `&` in a label is a mnemonic, and a literal ampersand is `&&`. The scale equation type is named `ScaleEquation`. Stop `Wideband.exe` before you replace it, because the running process locks the file. Compile to a temporary exe when you only need a type check. Relaunch with WMI `Win32_Process.Create` so the shell does not close the app at the end of the command.

Leave `C:\Program Files (x86)\VF Tuner` and `C:\Users\ryanp\VF-Tuner-backup` unchanged. `esp32.dll` in that install is only the J2534 cable library. Do not decrypt `BinDefs.enc`, `DTCDefs.enc`, or `Nsum.enc`. Do not unpack `vF Tuner.exe`. Do not read `.tblp` bodies.

Key masks, rotate counts, XOR masks, and add offsets come only from `security-config.json`. A row with `configured` false, or a missing row, fails closed and must not send a guessed key. The message is `No security key function is configured for {profile name}.` An all-zero seed is already unlocked and must not call the key function. The shipped JSON is all `configured: false` with zero placeholders. Do not copy a production mask into the program.

Do not add a checksum bypass, trouble-code clearing, or malfunction-lamp control. An `XDFFLAG` edit is a generic calibration flag. Do not call `PassThruSetProgrammingVoltage`, five-baud init, or periodic messages. K-Line fast init is `PassThruIoctl` id `0x05` (`FAST_INIT`) after connect. Ioctl id `0x03` is `READ_VBATT`. K-Line connect uses flag `0x200` (`ISO9141_NO_CHECKSUM`). CAN ids in `PASSTHRU_MSG` are big-endian: `0x7E8` is the bytes `00 00 07 E8`.

Read and flash ask for a file the user picks and a hex address the user types, then show a confirmation. Do not embed a sample firmware image or a default ECU address. Do not edit `sample-definition.json`. Its SHA-256 is `664FF498090DFFAA84BCA3B17CA607479553C6799DB8224044018DFC54EF3930`. Cell edits on an XDF document write the ROM buffer. `TuneDocument.Persist` is false for that document, and `Write` throws `This definition is not written to the sample file.`

Set `XAI_API_KEY` in the environment when the in-app assistant should call `https://api.x.ai/v1/responses` with model `grok-4.7`. With no key, the assistant answers from the local logger catalog. Leave that prompt as it is.

The cable page holds the J2534 device and channel open and sends tester present `0x3E` every 2 seconds. Profiles are Tacoma 2009+ CAN, Tacoma 2005-2008 K-Line, Silverado 2007+ 29-bit CAN, and Silverado 1999-2007 J1850 VPW. Read ECU uses service `0x23`. Write confirms, then runs `0x3E`, programming session `0x10`, security `0x27`, download `0x34`, transfer `0x36`, exit `0x37`, and reset `0x11`. KWP2000 request download is `34`, a 3-byte address, `00`, and a 3-byte size. Its `74` response is one max-block byte. Its `76` response may omit the sequence echo. UDS download framing stays the ISO 14229 form.

Calibrate loads a `.bin` or Toyota `.cuw` and a TunerPro `.xdf`. Category folders collapse. The find box filters by name, category, or unit. The grid runs from blue through green to red. A cell changed from the loaded base is bold and shows its delta. `+` and `-` step by the amount. The command box takes `+5%`, `-2%`, or `=value`. `I` interpolates the selection. Switches write a flag bit or a one-byte value into the ROM buffer. Save ROM and flash stamp a configured `sum16`, `sum32`, or `crc32` from `security-config.json`. With no range, Save ROM does not write and a flash stops before the download. The badge texts are exactly `Checksum Valid` and `Checksum Updated`.

Suggest VE updates the sample table while the sample is open. After a ROM and an XDF are open, it updates the table whose id is `ve` in the ROM buffer.

Erase stays off unless a caller asks for it. There is no RAM kernel upload and no `XDFPATCH` apply. No production key is configured. A transfer to a real ECU has not been run from this project.

| File | Role |
| --- | --- |
| `Wideband.cs` | Main form, Cable, Logger, Profiles, and the assistant |
| `Calibrate.cs` | Calibrate page, log page, and table math |
| `DefinitionEngine.cs` | `.bin`, `.cuw`, `.xdf`, and the ROM buffer |
| `CanLink.cs` | J2534 host process and the SocketCAN backend |
| `J2534Host.cs` | 32-bit PassThru helper |
| `EcuBus.cs` | Vehicle profiles and KWP/VPW frames |
| `IsoTp.cs` | ISO 15765-2 on classic CAN |
| `Uds.cs` | Diagnostic client and services |
| `SecuritySession.cs` | Session `0x10` and security `0x27` |
| `PlatformSecurityAndChecksum.cs` | Key math and checksums from JSON |
| `security-config.json` | Fail-closed key masks and checksum ranges |
| `FlashDownload.cs`, `FlashJob.cs`, `FlashWorker.cs` | Download sequence and progress |
| `LinkSession.cs`, `VehicleLink.cs` | The open cable |
| `MemoryMap.cs` | Caller-supplied address map |
| `device_check.ps1` | 32-bit probe of `esp32.dll` |
| `sample-definition.json` | Synthetic sample calibration |

## Using the app

Wideband is a Windows desktop app. Open `Wideband.exe`.

Calibrate opens a `.bin` or a Toyota `.cuw` calibration and a TunerPro `.xdf` definition. A `.cuw` file contributes its raw calibration payload; the text header is kept only as a note. The parameter tree groups `XDFTABLE`, `XDFCONSTANT`, and `XDFFLAG` items into collapsible category folders. The find box filters that tree by name, category, or unit. Each value is scaled with its equation, and an edit writes the raw bytes back into the ROM buffer. The grid colors cells from blue through green to red as the value runs from low to high. A cell changed from the loaded base is bold and shows its delta. `+` and `-` step the selection by the amount. The command box accepts `+5%`, `-2%`, or `=value`. `I` interpolates the selected block from its corners. Switches lists flags and one-byte switch rows; a toggle writes that bit or byte in the ROM buffer immediately. Save ROM writes that buffer to a file you choose. Until both files are open, Calibrate edits the sample definition, `sample-definition.json`, beside the program. The sample describes scalars, curves, and maps with units, axes, and a beginner, intermediate, or advanced level. Table tools add, scale, smooth, interpolate, and compare against the values loaded with the table. Trace highlights the nearest cell for an RPM and load you type.

Log opens a CSV and draws it, evaluates a channel formula such as `lambda / target`, and can apply a volumetric-efficiency suggestion. On the sample calibration that writes `sample-definition.json`. After a ROM and an XDF are open, it writes the table whose id is `ve` into the ROM buffer.

Logger reads `Logger.ini` from the vF Tuner install. Cable uses `J2534Host.exe`, a 32-bit helper, so the 64-bit app can load `esp32.dll`. Connect Cable opens the device, reads the version, and leaves the device id and channel id open. The Cable page selects a Toyota Tacoma or Chevrolet Silverado profile. Tacoma 2009+ uses ISO 15765 on 11-bit CAN at 500 kbps (`0x7E0` / `0x7E8`). Silverado 2007+ uses ISO 15765 on 29-bit CAN at 500 kbps (`0x18DA10F1` / `0x18DAF110`). Tacoma 2005–2008 uses ISO 14230 K-Line at 10400 baud. Connecting that profile sends fast initialization, start communication `81`, to the profile ECU address from the tester address. Silverado 1999–2007 uses J1850 VPW at 10400 baud. Tester present (service `0x3E`) is sent every 2 seconds while the channel is held. `SocketCanLink` is the Linux SocketCAN backend and refuses to open on Windows. `IsoTpLink` segments longer CAN messages: a single frame up to 7 data bytes, or a first frame, flow control with block size and STmin, and consecutive frames. Profiles lists the Toyota profile filenames. The profile bodies stay unread.

`ExtendedSessionAccess` requests a diagnostic session (service 0x10) and then security access (service 0x27). UDS programming uses session `0x02`. KWP2000 programming uses session `0x85`. It sends the seed request, keeps the seed bytes from the response, passes them to the key function for that profile, and sends that key. Timeouts and negative responses stop the sequence. An all-zero seed is already unlocked and does not call the key function.

`FlashDownload` sends Request Download (service 0x34) with the memory address and the uncompressed image length, then Transfer Data (service 0x36) in blocks, then Request Transfer Exit (service 0x37). Each block carries the next sequence counter, starting at 1 and wrapping after 255. The block size is the smaller of the caller's chunk, such as 256 or 1024, and the size the ECU reports. UDS encodes the address and size in the ISO 14229 header. KWP2000 sends `34`, a 3-byte address, `00`, and a 3-byte size. Its `74` response is one max-block byte, including the service id and sequence counter, and its `76` response may omit the sequence echo. A progress callback receives the bytes acknowledged and the total.

`VehicleLink` is the open cable. A response-pending negative response (`7F`, service, `78`) extends the read wait to at least 5 seconds while a sector is busy. Read ECU asks for an address and a length, confirms, then reads with service `0x23`. Write/Flash Calibration asks for a file and a hexadecimal address, confirms, then runs tester present, programming session `0x10`, security access `0x27`, download `0x34`, transfer `0x36`, exit `0x37`, and reset `0x11`. The progress bar and the communication log show the percent, the rate, and the service bytes. During service `0x27` the log also prints the incoming seed hex bytes and the calculated key hex bytes. The channel stays open when the transfer ends. Disconnect closes it. A failed transfer does not close the channel by itself. K-Line and VPW frames that do not fit the bus are rejected before they are sent. A VPW download can name at most 65535 bytes.

`MemoryImage` loads and saves a raw binary. It refuses `.enc` files and profiles whose first bytes are `VFTBLPE1`. `CalibrationFiles` does the same for `.bin` and, for `.cuw`, extracts Motorola S-records or a raw block that follows the text header. `XdfDocument` reads `XDFTABLE`, `XDFCONSTANT`, and `XDFFLAG`, including addresses, element size, signed and endian flags, floating point, axis labels, and the equation used to read and write each value. `ScalarCodec` and `TableBinding` convert uint8, int8, uint16, int16, uint32, int32, and float32 values with endianness, an optional bit field, and `raw * factor + offset`. `MemoryDefinition` reads that shape from JSON. `RegionChecksum` computes CRC-16/CCITT-FALSE, CRC-16/MODBUS, or CRC-32/ISO-HDLC over a caller-specified range and can stamp that field before a file is saved or a transfer is queued.

`PlatformSecurityAndChecksum` reads `security-config.json` beside the program. Toyota Tacoma profiles (`tacoma-can`, `tacoma-kline`) use the 32-bit Denso transform: each seed byte swaps its high and low nibble, then those four bytes are XOR-ed with the profile mask. Chevrolet Silverado profiles (`silverado-can`, `silverado-vpw`) use a GM row from `gmKeys`: rotate, XOR the mask, then add the offset, at 16 or 32 bits. The seed width has to match the row. Add a Denso operating-system mask as an object in `densoGenerations` and set the profile `generation` to that id. Add a GM operating-system key as an object in `gmKeys` and set the profile `keyId` to that id. Set `configured` to true only after the mask is one you are allowed to use. The shipped file leaves every row at `configured: false` and uses a zero placeholder, so a missing or inactive entry does not send a key.

Before Save ROM writes a `.bin`, and before service `0x34`, each configured checksum is stamped and checked. A `checksums` object names `sum16`, `sum32`, or `crc32`, an inclusive `start` and `end`, and a `store` address. An address is a file offset when it falls inside the image, and otherwise the address minus the ROM origin, the same rule the XDF loader uses. Bytes at `store` count as zero while the block is calculated, then the field receives that sum or CRC. `sum16` and `sum32` are the sum of each byte modulo 2^16 or 2^32. `crc32` is CRC-32/ISO-HDLC, and the ASCII bytes `123456789` produce `0xCBF43926`. A range with an empty `profile` is used for Save ROM and for every flash. A range with a profile id is used only when flashing that profile. If no range applies, Save ROM does not write the file, and a flash stops before the download. After a save, Calibrate shows `Checksum Valid` when the stamp left the buffer unchanged, or `Checksum Updated` when it changed bytes. Editing a table clears that badge when the buffer no longer matches.

The calibrate view groups parameters by domain. Changed cells show green when the value rose and red when it fell. Shift+Up and Shift+Down step the selection. Shift+Left and Shift+Right scale it. A map draws as a color-scaled surface beside the grid.

Wideband does not ship a production ECU key, suppress trouble codes, or open the encrypted vF Tuner definitions. Key masks and checksum windows stay in `security-config.json`.

Set `XAI_API_KEY` before starting Wideband if you want the assistant to call Grok. Without that key, the assistant answers from the logger file.
