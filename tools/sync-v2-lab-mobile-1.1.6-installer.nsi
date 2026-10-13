Unicode true
!include "MUI2.nsh"
Name "JARVIS ALTRON · Sync v2 LAB"
OutFile "..\artifacts\sync-v2-lab\JARVIS-1.1.0-Sync-v2-LAB-Mobile-1.1.6-setup.exe"
InstallDir "$LOCALAPPDATA\JARVIS-ALTRON-Sync-v2-LAB"
RequestExecutionLevel user
SetCompressor zlib
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "Russian"
!insertmacro MUI_LANGUAGE "English"
Function .onInit
  StrCpy $INSTDIR "$LOCALAPPDATA\JARVIS-ALTRON-Sync-v2-LAB"
FunctionEnd
Section "Sync v2 LAB"
  SetOutPath "$INSTDIR"
  File /r "..\artifacts\sync-v2-lab\windows-1.1.0-mobile-1.1.6\*"
  WriteUninstaller "$INSTDIR\Uninstall-Sync-v2-LAB.exe"
  CreateShortcut "$DESKTOP\JARVIS Sync v2 LAB.lnk" "$INSTDIR\JARVIS-Sync-v2-LAB.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\JARVIS-Sync-v2-LAB" "DisplayName" "JARVIS ALTRON Sync v2 LAB"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\JARVIS-Sync-v2-LAB" "UninstallString" '"$INSTDIR\Uninstall-Sync-v2-LAB.exe"'
SectionEnd
Section "Uninstall"
  StrCmp $INSTDIR "$LOCALAPPDATA\JARVIS-ALTRON-Sync-v2-LAB" 0 refused
  Delete "$DESKTOP\JARVIS Sync v2 LAB.lnk"
  Delete "$INSTDIR\JARVIS-Sync-v2-LAB.exe"
  Delete "$INSTDIR\jarvis-app-sync-v2-lab.exe"
  Delete "$INSTDIR\Uninstall-Sync-v2-LAB.exe"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\JARVIS-Sync-v2-LAB"
  ; Personal profiles and recovery copies are deliberately retained. No recursive deletion.
  Goto finished
  refused:
  MessageBox MB_ICONSTOP "Непредусмотренный каталог. Автоматическое удаление отменено."
  finished:
SectionEnd
