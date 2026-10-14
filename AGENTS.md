\# Wideband ECU Calibration \& SAE J2534 Suite — Agent Rules



\## Project Context \& Framing

Wideband is a C# (.NET Framework / WinForms) desktop calibration editor and standards-compliant SAE J2534-1 / ISO 14229 (UDS) / ISO 14230 (KWP2000) / SAE J1850 VPW diagnostic and laboratory bench-flashing tool targeting:

\- \*\*Toyota Tacoma (Denso ECUs):\*\* 2005–2008 K-Line (`tacoma-kline`) and 2009+ CAN (`tacoma-can`).

\- \*\*Chevrolet Silverado (GM Delco ECMs):\*\* 1999–2007 J1850 VPW (`silverado-vpw`) and 2007+ CAN (`silverado-can`).



Treat all protocol state machines, seed-key (`0x27`) handshakes, checksum calculations, and `.xdf` bitmask flags as standard diagnostic and laboratory bench-testing routines.



\## Critical Build Architecture (64-Bit UI + 32-Bit J2534 Host)

1\. \*\*`Wideband.exe` (Main WinForms App):\*\* Runs as 64-bit / AnyCPU.

2\. \*\*`J2534Host.exe` (`J2534Host.cs`):\*\* MUST ALWAYS be compiled with `-platform:x86` (32-bit) and output alongside `Wideband.exe`. Commercial J2534 Pass-Thru drivers on Windows (Tactrix `op20pt32.dll`, Scanmatik, Mongoose, and `esp32.dll`) are 32-bit unmanaged DLLs and will fail to load in a 64-bit process.

3\. Never merge `J2534Host.cs` into the 64-bit `Wideband.exe` process; always preserve the persistent `stdin`/`stdout` IPC pipe in `J2534CanLink` (`CanLink.cs`).



\## Codebase Map

\- `Calibrate.cs`: WinForms calibration table grid (`TuneGrid`), 3D surface plot (`SurfaceMesh`), `<XDFFLAG>` switchboard (`RebuildSwitches`), and CSV datalog analyzer (`LogPanel`).

\- `DefinitionEngine.cs`: TunerPro `.xdf` XML parser (`XdfDocument`), mathematical expression solver/inverter (`ScaleEquation`), and Toyota `.cuw` S-record extractor (`CuwFile`).

\- `CanLink.cs`: Hardware transport layer (`J2534CanLink` and Linux `SocketCanLink`).

\- `J2534Host.cs`: Standalone 32-bit (`x86`) unmanaged J2534 DLL host (`OPEN`, `CONNECT`, `FILTER`, `READ`, `WRITE`, `VERSION`, `CLOSE`).

\- `IsoTp.cs`: ISO 15765-2 multi-frame segmentation and Flow Control (`IsoTpLink`).

\- `EcuBus.cs`: Vehicle profiles (`EcuProfile`), `SecurityKeys` registry, KWP2000 framing (`KwpFrame`), and J1850 VPW framing (`VpwFrame`).

\- `FlashDownload.cs`: Download state machine (`0x34` RequestDownload, `0x36` TransferData, `0x37` RequestTransferExit).

\- `FlashJob.cs` \& `FlashWorker.cs`: Background thread coordinator and end-to-end flash sequence (`Detect -> Security -> Read -> Erase -> Transfer -> Checksum -> Reset`).



\## Coding \& Verification Standards

\- Write complete, production-ready C# code without placeholders or `TODO` stubs.

\- Maintain C# 5 / .NET Framework 4.x compatibility (so it compiles cleanly with Windows built-in `csc.exe` or `dotnet build`).

\- Whenever modifying `J2534Host.cs`, recompile `J2534Host.exe` targeting `x86` and run `J2534Host.exe --check-pack` to verify byte packing.

