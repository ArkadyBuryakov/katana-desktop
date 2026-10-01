; Katana Desktop: per-user Windows installer (no admin rights needed).
; Built by `make windows`:
;   makensis -DVERSION=<x.y.z[-rcN]> -DFILEVERSION=<x.y.z.0> -DSRCDIR=<dir with exe + dll> -DICON=<.ico> -DWIZARD=<.bmp> -DOUTFILE=<setup.exe> installer/setup.nsi

Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"
!include "x64.nsh"

!define APPNAME   "Katana Desktop"
!define EXENAME   "katana-desktop.exe"
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\KatanaDesktop"
; WebView2 Evergreen runtime client id, see learn.microsoft.com/microsoft-edge/webview2/concepts/distribution
!define WEBVIEW2_ID "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"

Name "${APPNAME}"
OutFile "${OUTFILE}"
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\Programs\${APPNAME}"
InstallDirRegKey HKCU "${UNINSTKEY}" "InstallLocation"
SetCompressor /SOLID lzma
BrandingText "${APPNAME} ${VERSION}"

VIProductVersion "${FILEVERSION}"
VIAddVersionKey "ProductName" "${APPNAME}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "FileDescription" "${APPNAME} Setup"
VIAddVersionKey "LegalCopyright" ""

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_WELCOMEFINISHPAGE_BITMAP "${WIZARD}"
!define MUI_UNWELCOMEFINISHPAGE_BITMAP "${WIZARD}"
!define MUI_ABORTWARNING
!define MUI_WELCOMEPAGE_TEXT "This will install ${APPNAME} ${VERSION}, an unofficial desktop client for Nonograms Katana user puzzles.$\r$\n$\r$\nNo administrator rights are needed: it installs for your user only."
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXENAME}"
!define MUI_FINISHPAGE_RUN_TEXT "Start ${APPNAME}"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

!macro CloseApp
  ; a running copy would keep the exe locked
  nsExec::Exec 'taskkill /F /IM ${EXENAME}'
  Pop $0
  Sleep 300
!macroend

Function .onInit
  ; WebView2 ships with Windows 10/11, but can be missing on stripped-down systems
  SetRegView 64
  ReadRegStr $0 HKLM "SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\${WEBVIEW2_ID}" "pv"
  ${If} $0 == ""
    ReadRegStr $0 HKCU "Software\Microsoft\EdgeUpdate\Clients\${WEBVIEW2_ID}" "pv"
  ${EndIf}
  SetRegView 32
  ${If} $0 == ""
  ${OrIf} $0 == "0.0.0.0"
    MessageBox MB_YESNO|MB_ICONEXCLAMATION "${APPNAME} needs the Microsoft Edge WebView2 Runtime, which doesn't seem to be installed.$\r$\n$\r$\nOpen the download page now? (Setup will continue either way.)" /SD IDNO IDNO skip_webview2
    ExecShell "open" "https://developer.microsoft.com/microsoft-edge/webview2/"
  ${EndIf}
  skip_webview2:
FunctionEnd

Section "${APPNAME}" SecApp
  SectionIn RO
  !insertmacro CloseApp
  SetOutPath "$INSTDIR"
  File "${SRCDIR}\${EXENAME}"
  File "${SRCDIR}\WebView2Loader.dll"
  Delete "$INSTDIR\uninstall.ps1" ; left by the older PowerShell install script
  WriteUninstaller "$INSTDIR\uninstall.exe"

  CreateShortcut "$SMPROGRAMS\${APPNAME}.lnk" "$INSTDIR\${EXENAME}" "" "$INSTDIR\${EXENAME}" 0 SW_SHOWNORMAL "" "Nonograms Katana puzzles"

  WriteRegStr HKCU "${UNINSTKEY}" "DisplayName" "${APPNAME}"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayIcon" "$INSTDIR\${EXENAME},0"
  WriteRegStr HKCU "${UNINSTKEY}" "Publisher" "${APPNAME}"
  WriteRegStr HKCU "${UNINSTKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTKEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINSTKEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKCU "${UNINSTKEY}" "EstimatedSize" "$0"
SectionEnd

Section "Desktop shortcut" SecDesktop
  CreateShortcut "$DESKTOP\${APPNAME}.lnk" "$INSTDIR\${EXENAME}" "" "$INSTDIR\${EXENAME}" 0 SW_SHOWNORMAL "" "Nonograms Katana puzzles"
SectionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecApp} "The application (about 4 MB)."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} "Put a ${APPNAME} shortcut on the desktop."
!insertmacro MUI_FUNCTION_DESCRIPTION_END

Section "Uninstall"
  !insertmacro CloseApp
  Delete "$INSTDIR\${EXENAME}"
  Delete "$INSTDIR\WebView2Loader.dll"
  Delete "$INSTDIR\uninstall.ps1"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APPNAME}.lnk"
  Delete "$DESKTOP\${APPNAME}.lnk"
  DeleteRegKey HKCU "${UNINSTKEY}"
  DetailPrint "Your progress and login are kept in $LOCALAPPDATA\katana-desktop"
SectionEnd
