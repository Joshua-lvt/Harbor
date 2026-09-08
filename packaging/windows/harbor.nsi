; Harbor Windows installer (NSIS 3, MUI2).
;
; Built by CI (.github/workflows/build.yml) after windeployqt:
;   makensis /DVERSION=2.1.0 /DSTAGEDIR=dist\harbor-windows-x86_64
;            /DDEPSDIR=dist\win-deps /DOUTFILE=harbor-windows-setup.exe
;            packaging\windows\harbor.nsi
;
; What it does, in order: Harbor files, Start Menu entries, the official
; Microsoft Visual C++ 2022 redistributable (chained, quiet — Qt requires
; the official package, never side-by-side DLLs), desktop icon (optional),
; and an uninstaller with Add/Remove Programs registration.
;
; What it deliberately does NOT do: bundle or install any network client —
; Harbor connects over the user's own internet (direct IPv6/IPv4, relay
; fallback), remove the C++ runtime on uninstall (shared system
; component), or touch the user's Harbor state/identity in
; %LOCALAPPDATA%.
Unicode true
RequestExecutionLevel admin
SetCompressor /SOLID lzma
ManifestDPIAware true

!include "LogicLib.nsh"

!ifndef VERSION
  !define VERSION "0.0.0-dev"
!endif
!ifndef STAGEDIR
  !define STAGEDIR "dist\harbor-windows-x86_64"
!endif
!ifndef DEPSDIR
  !define DEPSDIR "dist\win-deps"
!endif
!ifndef OUTFILE
  !define OUTFILE "harbor-windows-setup.exe"
!endif
!ifndef BROKER
  !define BROKER "build\harbor-update-broker.exe"
!endif

Name "Harbor ${VERSION}"
OutFile "${OUTFILE}"
InstallDir "$PROGRAMFILES64\Harbor"
InstallDirRegKey HKLM "Software\Harbor" "InstallDir"
Icon "..\icons\harbor.ico"
UninstallIcon "..\icons\harbor.ico"
VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "Harbor"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "CompanyName" "Harbor"
VIAddVersionKey "FileDescription" "Harbor installer"
VIAddVersionKey "LegalCopyright" "MIT OR Apache-2.0"

!include "MUI2.nsh"
!define MUI_ABORTWARNING
!define MUI_ICON "..\icons\harbor.ico"
!define MUI_UNICON "..\icons\harbor.ico"
!define MUI_HEADERIMAGE
!define MUI_HEADERIMAGE_BITMAP "header.bmp"
!define MUI_WELCOMEFINISHPAGE_BITMAP "wizard.bmp"
!define MUI_UNWELCOMEFINISHPAGE_BITMAP "wizard.bmp"
; No auto-run on Finish: the installer is elevated, and launching Harbor
; from it would create its identity/state as Administrator instead of the
; user. The Start Menu entry opens it unprivileged.
!define MUI_FINISHPAGE_TEXT "Harbor is installed. Open it from the Start Menu — it connects to your server over your own internet, with no extra software."
!define MUI_FINISHPAGE_NOAUTOCLOSE
!define MUI_UNFINISHPAGE_NOAUTOCLOSE

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_UNPAGE_FINISH
!insertmacro MUI_LANGUAGE "PortugueseBR"
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  ; Explicit temp-dir init across NSIS versions; $PLUGINSDIR is removed
  ; automatically when the installer exits.
  InitPluginsDir
FunctionEnd

Section "Harbor (required)" SectionMain
  SectionIn RO
  SetOutPath "$INSTDIR"
  File /r "${STAGEDIR}\*.*"

  ; Third-party installers travel INSIDE this setup: a compile-time CI path
  ; does not exist on the user's PC (referencing it fails with msiexec 1619
  ; / missing-file errors). Extract to the temp plugins dir (auto-removed
  ; after install) and chain from there. A missing source fails the build
  ; here instead of failing silently on the user's machine.
  SetOutPath "$PLUGINSDIR"
  File "${DEPSDIR}\vc_redist.x64.exe"
  SetOutPath "$INSTDIR"

  SetOutPath "$PROGRAMFILES64\Harbor Update"
  File /oname=harbor-update-broker.exe "${BROKER}"
  WriteUninstaller "$PROGRAMFILES64\Harbor Update\uninstall.exe"
  SetOutPath "$INSTDIR"
  WriteRegStr HKLM "Software\Harbor" "InstallDir" "$INSTDIR"
  WriteRegStr HKLM "Software\Harbor" "BrokerPath" "$PROGRAMFILES64\Harbor Update\harbor-update-broker.exe"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Harbor" \
    "DisplayName" "Harbor ${VERSION}"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Harbor" \
    "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Harbor" \
    "Publisher" "Harbor"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Harbor" \
    "DisplayIcon" "$INSTDIR\harbor.exe"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Harbor" \
    "UninstallString" "$PROGRAMFILES64\Harbor Update\uninstall.exe"
  WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Harbor" \
    "NoModify" 1
  WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Harbor" \
    "NoRepair" 1

  CreateDirectory "$SMPROGRAMS\Harbor"
  CreateShortcut "$SMPROGRAMS\Harbor\Harbor.lnk" "$INSTDIR\harbor.exe" "" \
    "$INSTDIR\harbor.exe" 0
  CreateShortcut "$SMPROGRAMS\Harbor\Uninstall Harbor.lnk" "$PROGRAMFILES64\Harbor Update\uninstall.exe"

  ; Official VC++ 2022 runtime (Qt requires the redistributable package).
  ; Quiet, idempotent: a present runtime returns success immediately.
  DetailPrint "Installing Microsoft Visual C++ 2022 runtime..."
  ExecWait '"$PLUGINSDIR\vc_redist.x64.exe" /quiet /norestart' $0
  ${If} $0 != 0
  ${AndIf} $0 != 1638
  ${AndIf} $0 != 3010
    MessageBox MB_ICONEXCLAMATION|MB_OK \
      "The Visual C++ runtime installer returned code $0. Harbor may fail to start; reinstalling it manually usually fixes this."
  ${EndIf}
SectionEnd

Section /o "Desktop icon" SectionDesktopIcon
  CreateShortcut "$DESKTOP\Harbor.lnk" "$INSTDIR\harbor.exe" "" \
    "$INSTDIR\harbor.exe" 0
SectionEnd

Section "Uninstall"
  Delete "$SMPROGRAMS\Harbor\Harbor.lnk"
  Delete "$SMPROGRAMS\Harbor\Uninstall Harbor.lnk"
  RMDir "$SMPROGRAMS\Harbor"
  Delete "$DESKTOP\Harbor.lnk"
  ; Application files only: identity/state in %LOCALAPPDATA% survives so a
  ; reinstall keeps working, and the C++ runtime (shared system component)
  ; is not removed.
  RMDir /r "$INSTDIR"
  RMDir /r "$PROGRAMFILES64\Harbor Update"
  DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Harbor"
  DeleteRegKey HKLM "Software\Harbor"
SectionEnd
