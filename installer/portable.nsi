; Katana Desktop: single-file portable launcher, nothing gets installed.
; MinGW builds need WebView2Loader.dll next to the exe, so this carries both, unpacks them
; to a temporary folder (removed again when the app exits) and runs the app from there.
; Progress and login live in %LOCALAPPDATA%\katana-desktop, shared with an installed copy.
; Built by `make windows`:
;   makensis -DVERSION=<x.y.z> -DSRCDIR=<dir with exe + dll> -DICON=<.ico> -DOUTFILE=<portable.exe> installer/portable.nsi

Unicode true
!include "FileFunc.nsh"

!define APPNAME "Katana Desktop"
!define EXENAME "katana-desktop.exe"

Name "${APPNAME}"
OutFile "${OUTFILE}"
Icon "${ICON}"
RequestExecutionLevel user
SilentInstall silent
SetCompressor /SOLID lzma

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APPNAME}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "FileDescription" "${APPNAME} (portable)"
VIAddVersionKey "LegalCopyright" ""

Section
  InitPluginsDir
  SetOutPath "$PLUGINSDIR\app"
  File "${SRCDIR}\${EXENAME}"
  File "${SRCDIR}\WebView2Loader.dll"
  ${GetParameters} $1
  SetOutPath "$EXEDIR" ; run with the launcher's folder as working directory
  ExecWait '"$PLUGINSDIR\app\${EXENAME}" $1' $0
  SetErrorLevel $0
  ; $PLUGINSDIR is deleted automatically when the launcher exits
SectionEnd
