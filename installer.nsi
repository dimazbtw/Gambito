; Xadrez installer (NSIS). Per-user install: no administrator rights needed.
; Build:  cargo build --release   then   makensis installer.nsi   ->   dist\Xadrez-Setup.exe
; Override the paths with  /DEXE=<path to xadrez.exe>  /DOUT=<installer path>  if needed.
!ifndef EXE
  !define EXE "target\release\xadrez.exe"
!endif
!ifndef OUT
  !define OUT "dist\Xadrez-Setup.exe"
!endif
Unicode true
!define APP "Xadrez"
!define VERSION "1.0.0"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP}"

!include "MUI2.nsh"

Name "${APP}"
OutFile "${OUT}"
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\Programs\${APP}"
InstallDirRegKey HKCU "Software\${APP}" "InstallDir"
; the exe already carries compressed engines, so a fast compressor is enough
SetCompressor /FINAL zlib
VIProductVersion "1.0.0.0"
VIAddVersionKey "ProductName" "${APP}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "GPL-3.0"
VIAddVersionKey "FileDescription" "Instalador do ${APP}"

!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\Xadrez.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Abrir o ${APP}"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "Portuguese"

Section "Instalar"
  SetOutPath "$INSTDIR"
  File "/oname=Xadrez.exe" "${EXE}"
  WriteUninstaller "$INSTDIR\Desinstalar.exe"

  CreateDirectory "$SMPROGRAMS\${APP}"
  CreateShortcut "$SMPROGRAMS\${APP}\${APP}.lnk" "$INSTDIR\Xadrez.exe"
  CreateShortcut "$SMPROGRAMS\${APP}\Desinstalar ${APP}.lnk" "$INSTDIR\Desinstalar.exe"
  CreateShortcut "$DESKTOP\${APP}.lnk" "$INSTDIR\Xadrez.exe"

  WriteRegStr HKCU "Software\${APP}" "InstallDir" "$INSTDIR"
  ; entry in "Apps instaladas"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${APP}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "${APP}"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\Xadrez.exe"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\Desinstalar.exe"'
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\Desinstalar.exe" /S'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
  SectionGetSize 0 $0
  WriteRegDWORD HKCU "${UNINST_KEY}" "EstimatedSize" $0
SectionEnd

Section "Uninstall"
  Delete "$INSTDIR\Xadrez.exe"
  Delete "$INSTDIR\Desinstalar.exe"
  RMDir "$INSTDIR"
  Delete "$DESKTOP\${APP}.lnk"
  Delete "$SMPROGRAMS\${APP}\${APP}.lnk"
  Delete "$SMPROGRAMS\${APP}\Desinstalar ${APP}.lnk"
  RMDir "$SMPROGRAMS\${APP}"
  ; engines the app unpacked into %TEMP% on first use
  Delete "$TEMP\xadrez-*.exe"
  Delete "$TEMP\xadrez-*.part"
  ; user data (added engines, learned piece shapes): ask first
  IfFileExists "$APPDATA\Xadrez\*.*" 0 +3
    MessageBox MB_YESNO|MB_ICONQUESTION "Apagar também as definições (engines adicionadas e peças aprendidas)?" /SD IDNO IDNO +2
    RMDir /r "$APPDATA\Xadrez"
  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegKey HKCU "Software\${APP}"
SectionEnd
