Unicode true
!include "MUI2.nsh"
Name "JARVIS ALTRON"
OutFile "..\artifacts\release-1.1.16\JARVIS-ALTRON-1.1.16-Windows-x64-setup.exe"
; Existing install directory and identity retained for compatible upgrades.
InstallDir "$LOCALAPPDATA\JARVIS-ALTRON-Sync-v2-LAB"
RequestExecutionLevel user
SetCompressor zlib
VIProductVersion "1.1.16.0"
VIAddVersionKey "ProductName" "JARVIS ALTRON"
VIAddVersionKey "FileDescription" "JARVIS ALTRON 1.1.16 Setup"
VIAddVersionKey "FileVersion" "1.1.16"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "Russian"
!insertmacro MUI_LANGUAGE "English"
Section "JARVIS ALTRON"
  SetOutPath "$INSTDIR"
  File /r "..\artifacts\release-1.1.16\windows\*"
  WriteUninstaller "$INSTDIR\Uninstall-JARVIS.exe"
  CreateShortcut "$DESKTOP\JARVIS ALTRON.lnk" "$INSTDIR\JARVIS-Sync-v2-LAB.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\JARVIS-Sync-v2-LAB" "DisplayName" "JARVIS ALTRON"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\JARVIS-Sync-v2-LAB" "DisplayVersion" "1.1.16"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\JARVIS-Sync-v2-LAB" "UninstallString" '"$INSTDIR\Uninstall-JARVIS.exe"'
SectionEnd
Section "Uninstall"
  StrCmp $INSTDIR "$LOCALAPPDATA\JARVIS-ALTRON-Sync-v2-LAB" 0 refused
  Delete "$DESKTOP\JARVIS ALTRON.lnk"
  Delete "$DESKTOP\JARVIS Sync v2 LAB.lnk"
  Delete "$INSTDIR\JARVIS-Sync-v2-LAB.exe"
  Delete "$INSTDIR\jarvis-app-sync-v2-lab.exe"
  Delete "$INSTDIR\Uninstall-JARVIS.exe"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\JARVIS-Sync-v2-LAB"
  ; Retain personal data and recovery copies. No recursive deletion.
  Goto finished
  refused:
  MessageBox MB_ICONSTOP "Каталог установки не совпадает. Удаление отменено."
  finished:
SectionEnd
