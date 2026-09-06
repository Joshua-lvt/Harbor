; Harbor Windows installer (NSIS 3, MUI2).
;
; Built by CI (.github/workflows/build.yml) after windeployqt:
;   makensis /DVERSION=2.1.0 /DSTAGEDIR=dist\harbor-windows-x86_64
;            /DDEPSDIR=dist\win-deps /DOUTFILE=harbor-windows-setup.exe
;            packaging\windows\harbor.nsi
;
; What it does, in order: Harbor files, Start Menu entries, the official
; Microsoft Visual C++ 2022 redistributable (chained, quiet — Qt requires
; the official package, never side-by-side DLLs), the official Tailscale
; client (skipped when already present), desktop icon (optional), and an
; uninstaller with Add/Remove Programs registration.
;
; What it deliberately does NOT do: join the Tailnet (the app joins itself
; on first launch with its embedded key — native/HarborTailnet), remove
; Tailscale or the C++ runtime on uninstall (shared system components), or
; touch the user's Harbor state/identity in %LOCALAPPDATA%.
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
!define MUI_FINISHPAGE_TEXT "Harbor is installed. Open it from the Start Menu — it joins your private network by itself on first launch."
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

Section "Harbor (required)" SectionMain
  SectionIn RO
  SetOutPath "$INSTDIR"
  File /r "${STAGEDIR}\*.*"

  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKLM "Software\Harbor" "InstallDir" "$INSTDIR"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Harbor" \
    "DisplayName" "Harbor ${VERSION}"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Harbor" \
    "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Harbor" \
    "Publisher" "Harbor"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Harbor" \
    "DisplayIcon" "$INSTDIR\harbor.exe"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Harbor" \
    "UninstallString" "$INSTDIR\uninstall.exe"
  WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Harbor" \
    "NoModify" 1
  WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Harbor" \
    "NoRepair" 1

  CreateDirectory "$SMPROGRAMS\Harbor"
  CreateShortcut "$SMPROGRAMS\Harbor\Harbor.lnk" "$INSTDIR\harbor.exe" "" \
    "$INSTDIR\harbor.exe" 0
  CreateShortcut "$SMPROGRAMS\Harbor\Uninstall Harbor.lnk" "$INSTDIR\uninstall.exe"

  ; Official VC++ 2022 runtime (Qt requires the redistributable package).
  ; Quiet, idempotent: a present runtime returns success immediately.
  DetailPrint "Installing Microsoft Visual C++ 2022 runtime..."
  ExecWait '"${DEPSDIR}\vc_redist.x64.exe" /quiet /norestart' $0
  ${If} $0 != 0
  ${AndIf} $0 != 1638
  ${AndIf} $0 != 3010
    MessageBox MB_ICONEXCLAMATION|MB_OK \
      "The Visual C++ runtime installer returned code $0. Harbor may fail to start; reinstalling it manually usually fixes this."
  ${EndIf}

  ; Official Tailscale client, skipped when already present. Harbor joins
  ; the Tailnet itself on first launch; the tray app stays visible as the
  ; manual fallback (login window) if the automatic join ever fails.
  ${If} ${FileExists} "$PROGRAMFILES64\Tailscale\tailscale.exe"
    DetailPrint "Tailscale already installed, skipping."
  ${ElseIf} ${FileExists} "$PROGRAMFILES\Tailscale\tailscale.exe"
    DetailPrint "Tailscale already installed, skipping."
  ${Else}
    DetailPrint "Installing Tailscale (silent)..."
    ExecWait 'msiexec /i "${DEPSDIR}\tailscale.msi" /quiet /norestart' $1
    ${If} $1 != 0
    ${AndIf} $1 != 1638
    ${AndIf} $1 != 3010
      MessageBox MB_ICONEXCLAMATION|MB_OK \
        "The Tailscale installer returned code $1. Install it manually from https://tailscale.com/download, then open Harbor."
    ${EndIf}
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
  ; reinstall keeps working, and neither Tailscale nor the C++ runtime
  ; (shared system components) is removed.
  RMDir /r "$INSTDIR"
  DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Harbor"
  DeleteRegKey HKLM "Software\Harbor"
SectionEnd
