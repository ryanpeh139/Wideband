; Wideband Windows installer (NSIS 3).
; Build with scripts/build-windows-installer.sh, which stages the release and runs:
;   makensis -DSTAGE=<staged folder> -DVERSION=<version> -DOUTFILE=<setup.exe> installer/wideband.nsi
;
; Installs per user under %LOCALAPPDATA%\Programs\Wideband, so no administrator prompt is needed
; and Wideband can write its records (ECU snapshots, audit log, preferences) beside itself.
; Running a newer installer updates in place; security-config.json and user records are kept.

Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"

!ifndef STAGE
  !error "Pass -DSTAGE=<folder with the staged Windows release>"
!endif
!ifndef VERSION
  !define VERSION "0.0.0"
!endif
!ifndef OUTFILE
  !define OUTFILE "Wideband-Setup.exe"
!endif

!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Wideband"

Name "Wideband"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\Wideband"
InstallDirRegKey HKCU "Software\Wideband" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
BrandingText "Wideband ${VERSION}"

!define MUI_ABORTWARNING
!define MUI_WELCOMEPAGE_TEXT "This installs or updates Wideband ${VERSION}.$\r$\n$\r$\nIf Wideband is open, click Disconnect and close it before continuing.$\r$\n$\r$\nYour security-config.json and saved ECU records are kept when updating."
!define MUI_FINISHPAGE_RUN "$INSTDIR\wideband.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Start Wideband"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

; A running executable cannot be opened for writing, which is how a running Wideband is detected.
!macro CHECK_NOT_RUNNING PREFIX
Function ${PREFIX}CheckNotRunning
  ${If} ${FileExists} "$INSTDIR\wideband.exe"
    retry:
    ClearErrors
    FileOpen $0 "$INSTDIR\wideband.exe" a
    ${If} ${Errors}
      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "Wideband is running.$\r$\n$\r$\nClick Disconnect in Wideband, close it, then click Retry." IDRETRY retry
      Abort
    ${EndIf}
    FileClose $0
  ${EndIf}
FunctionEnd
!macroend
!insertmacro CHECK_NOT_RUNNING ""
!insertmacro CHECK_NOT_RUNNING "un."

Section "Wideband" SecMain
  SectionIn RO
  Call CheckNotRunning

  SetOutPath "$INSTDIR"
  SetOverwrite on
  File "${STAGE}\wideband.exe"
  File "${STAGE}\J2534Host.exe"
  File "${STAGE}\flash-layout.json"
  File "${STAGE}\telemetry-channels.json"
  File "${STAGE}\strategy-managers.json"
  File "${STAGE}\device_check.ps1"
  File "${STAGE}\Friday-CablePrep.ps1"
  File "${STAGE}\Tacoma-Setup-Guide.md"
  File "${STAGE}\README-FIRST.txt"

  ; Keys and checksum ranges are the user's to configure; an update never replaces them.
  SetOverwrite off
  File "${STAGE}\security-config.json"
  SetOverwrite on

  ; Shipped packs are replaced whole so files removed from a release do not linger.
  RMDir /r "$INSTDIR\profiles"
  RMDir /r "$INSTDIR\definition-packs"
  File /r "${STAGE}\profiles"
  File /r "${STAGE}\definition-packs"

  ; Shortcuts start in the install folder, where Wideband keeps its records.
  SetOutPath "$INSTDIR"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateDirectory "$SMPROGRAMS\Wideband"
  CreateShortCut "$SMPROGRAMS\Wideband\Wideband.lnk" "$INSTDIR\wideband.exe"
  CreateShortCut "$SMPROGRAMS\Wideband\Wideband (raw CAN mode).lnk" "$INSTDIR\wideband.exe" "--raw-can"
  CreateShortCut "$SMPROGRAMS\Wideband\Uninstall Wideband.lnk" "$INSTDIR\Uninstall.exe"
  CreateShortCut "$DESKTOP\Wideband.lnk" "$INSTDIR\wideband.exe"

  WriteRegStr HKCU "Software\Wideband" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "Wideband"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "Wideband"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\wideband.exe"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" "$\"$INSTDIR\Uninstall.exe$\""
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  Call un.CheckNotRunning

  Delete "$INSTDIR\wideband.exe"
  Delete "$INSTDIR\J2534Host.exe"
  Delete "$INSTDIR\flash-layout.json"
  Delete "$INSTDIR\telemetry-channels.json"
  Delete "$INSTDIR\strategy-managers.json"
  Delete "$INSTDIR\device_check.ps1"
  Delete "$INSTDIR\Friday-CablePrep.ps1"
  Delete "$INSTDIR\Tacoma-Setup-Guide.md"
  Delete "$INSTDIR\README-FIRST.txt"
  RMDir /r "$INSTDIR\profiles"
  RMDir /r "$INSTDIR\definition-packs"
  Delete "$INSTDIR\Uninstall.exe"
  Delete "$INSTDIR\j2534-trace.log"
  ; security-config.json, ECU records, and logs stay; the folder is removed only when empty.
  RMDir "$INSTDIR"

  Delete "$SMPROGRAMS\Wideband\Wideband.lnk"
  Delete "$SMPROGRAMS\Wideband\Wideband (raw CAN mode).lnk"
  Delete "$SMPROGRAMS\Wideband\Uninstall Wideband.lnk"
  RMDir "$SMPROGRAMS\Wideband"
  Delete "$DESKTOP\Wideband.lnk"

  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegKey HKCU "Software\Wideband"
SectionEnd
