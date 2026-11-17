Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
Name "JARVIS ALTRON"
OutFile "..\artifacts\release-1.1.19\JARVIS-ALTRON-1.1.19-Windows-x64-setup.exe"
; Existing install directory and identity retained for compatible upgrades.
InstallDir "$LOCALAPPDATA\JARVIS-ALTRON-Sync-v2-LAB"
RequestExecutionLevel user
SetCompressor zlib
VIProductVersion "1.1.19.0"
VIAddVersionKey "ProductName" "JARVIS ALTRON"
VIAddVersionKey "FileDescription" "JARVIS ALTRON 1.1.19 Setup"
VIAddVersionKey "FileVersion" "1.1.19"
!define WEBVIEW2_KEY "SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "Russian"
!insertmacro MUI_LANGUAGE "English"
Section "WebView2"
  ; The JARVIS window needs Microsoft Edge WebView2; install it only when it is missing.
  ReadRegStr $0 HKLM "${WEBVIEW2_KEY}" "pv"
  ${If} $0 == ""
  ${OrIf} $0 == "0.0.0.0"
    ReadRegStr $0 HKCU "${WEBVIEW2_KEY}" "pv"
  ${EndIf}
  ${If} $0 == ""
  ${OrIf} $0 == "0.0.0.0"
    DetailPrint "Устанавливаю Microsoft Edge WebView2…"
    InitPluginsDir
    File "/oname=$PLUGINSDIR\MicrosoftEdgeWebview2Setup.exe" "..\artifacts\release-1.1.19\MicrosoftEdgeWebview2Setup.exe"
    ExecWait '"$PLUGINSDIR\MicrosoftEdgeWebview2Setup.exe" /silent /install' $1
    ${If} $1 != 0
      MessageBox MB_ICONEXCLAMATION "Не удалось установить Microsoft Edge WebView2 (код $1). Установите его с сайта Microsoft, иначе окно JARVIS не откроется."
    ${EndIf}
  ${EndIf}
SectionEnd
Section "JARVIS ALTRON"
  SetOutPath "$INSTDIR"
  File /r "..\artifacts\release-1.1.19\windows\*"
  WriteUninstaller "$INSTDIR\Uninstall-JARVIS.exe"
  CreateShortcut "$DESKTOP\JARVIS ALTRON.lnk" "$INSTDIR\JARVIS-Sync-v2-LAB.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\JARVIS-Sync-v2-LAB" "DisplayName" "JARVIS ALTRON"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\JARVIS-Sync-v2-LAB" "DisplayVersion" "1.1.19"
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
  ; Retain personal data, downloaded components and recovery copies. No recursive deletion.
  Goto finished
  refused:
  MessageBox MB_ICONSTOP "Каталог установки не совпадает. Удаление отменено."
  finished:
SectionEnd
