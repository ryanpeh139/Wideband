WIDEBAND FOR WINDOWS
====================

INSTALL OR UPDATE
   Run Wideband-Setup.exe. It installs to
     %LOCALAPPDATA%\Programs\Wideband
   and adds Start menu and Desktop shortcuts. No administrator rights needed.
   To update later, run the newer Wideband-Setup.exe the same way; your
   security-config.json and saved ECU records are kept.

   Windows SmartScreen may say "Windows protected your PC" because the
   installer is not code-signed. Click "More info", then "Run anyway".

FIRST USE
   1. Close Techstream (only one program can use the cable at a time).
   2. Start Wideband from the Desktop shortcut. A console window opens beside
      it with log messages; leave it open.
   3. Pick your cable in the device list at the top right, for example
      "[USB J2534] MVCI Autokent-X64 - 1.4.3", and the vehicle
      "Toyota Tacoma 2009+ (Denso CAN)". Click Connect.
   4. Advanced > Flash & Diag > Identify ECU.
   Details: Tacoma-Setup-Guide.md in the install folder.

OPTIONAL CHECKS (PowerShell, in the install folder)
   Set-ExecutionPolicy -Scope Process Bypass
   .\device_check.ps1 -DllPath "<FunctionLibrary path of your cable>"

SAFETY
   security-config.json ships with every key OFF, so Wideband will not flash
   anything. Leave it that way. Use Techstream for Toyota updates.

UNINSTALL
   Settings > Apps > Wideband > Uninstall. Your records and
   security-config.json stay in the install folder.
