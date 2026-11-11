; Native per-user installer. Preserve the existing Iceon NSIS installation identity.
Unicode true
!include "MUI2.nsh"
!include "FileFunc.nsh"
!include "LogicLib.nsh"

!ifndef VERSION
  !error "VERSION required"
!endif
!ifndef PAYLOAD
  !error "PAYLOAD required"
!endif
!ifndef OUTPUT
  !error "OUTPUT required"
!endif
!ifndef LICENSE_FILE
  !error "LICENSE_FILE required"
!endif
!ifndef ICON_FILE
  !error "ICON_FILE required"
!endif

Name "Iceon"
OutFile "${OUTPUT}"
InstallDir "$LOCALAPPDATA\Iceon"
RequestExecutionLevel user
SetCompressor /SOLID lzma
Icon "${ICON_FILE}"
UninstallIcon "${ICON_FILE}"
VIProductVersion "${VERSION}.0"
VIAddVersionKey /LANG=1033 "ProductName" "Iceon"
VIAddVersionKey /LANG=1033 "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=1033 "ProductVersion" "${VERSION}"
VIAddVersionKey /LANG=1033 "FileDescription" "Iceon Setup"
VIAddVersionKey /LANG=1033 "LegalCopyright" "Iceon contributors"

!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Iceon"
; The previous installer derived its manufacturer from the Cargo author.
!define LEGACYKEY "Software\Yanuar\Iceon"
!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${LICENSE_FILE}"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\iceon.exe"
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  SetShellVarContext current
  SetRegView 32
  ReadRegStr $0 HKCU "${LEGACYKEY}" ""
  ${If} $0 != ""
    StrCpy $INSTDIR $0
  ${Else}
    ReadRegStr $0 HKCU "${UNINSTKEY}" "InstallLocation"
    ${If} $0 != ""
      StrCpy $1 $0 1
      ${If} $1 == '$\"'
        StrCpy $0 $0 -1 1
      ${EndIf}
      StrCpy $INSTDIR $0
    ${EndIf}
  ${EndIf}
FunctionEnd

Section "Iceon" SEC_MAIN
  SetOutPath "$INSTDIR"
  SetOverwrite try
  ; Quiet updates may overlap the exiting old process. Retry boundedly, never kill
  ; an unrelated process or report success while the executable is still locked.
  StrCpy $1 0
  retry_payload:
    ClearErrors
    File /oname=iceon.exe "${PAYLOAD}"
    IfErrors payload_locked payload_ready
  payload_locked:
    IntOp $1 $1 + 1
    ${If} $1 < 30
      Sleep 1000
      Goto retry_payload
    ${EndIf}
    SetErrorLevel 1
    Abort "Close Iceon and run Setup again."
  payload_ready:
  ClearErrors
  File /oname=LICENSE "${LICENSE_FILE}"
  File /oname=icon.ico "${ICON_FILE}"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKCU "${LEGACYKEY}" "" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayName" "Iceon"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTKEY}" "Publisher" "Iceon contributors"
  WriteRegStr HKCU "${UNINSTKEY}" "MainBinaryName" "iceon.exe"
  WriteRegStr HKCU "${UNINSTKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayIcon" '$\"$INSTDIR\icon.ico$\"'
  WriteRegStr HKCU "${UNINSTKEY}" "UninstallString" '$\"$INSTDIR\uninstall.exe$\"'
  WriteRegStr HKCU "${UNINSTKEY}" "QuietUninstallString" '$\"$INSTDIR\uninstall.exe$\" /S'
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoRepair" 1
  CreateShortcut "$SMPROGRAMS\Iceon.lnk" "$INSTDIR\iceon.exe" "" "$INSTDIR\icon.ico"
  IfErrors install_failed install_done
  install_failed:
    SetErrorLevel 1
    Abort "Setup could not finish writing files or registration."
  install_done:
SectionEnd

Function .onInstSuccess
  ; Match the existing updater's /S /UPDATE /R /ARGS restart contract.
  ${GetParameters} $0
  ClearErrors
  ${GetOptions} $0 "/R" $1
  ${IfNot} ${Errors}
    ${GetOptions} $0 "/ARGS" $2
    Exec '$\"$INSTDIR\iceon.exe$\" $2'
  ${EndIf}
FunctionEnd

Function un.onInit
  SetShellVarContext current
  SetRegView 32
FunctionEnd

Section "Uninstall"
  ClearErrors
  Delete "$INSTDIR\iceon.exe"
  IfErrors uninstall_failed
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\icon.ico"
  Delete "$INSTDIR\uninstall.exe"
  Delete "$SMPROGRAMS\Iceon.lnk"
  DeleteRegKey HKCU "${UNINSTKEY}"
  DeleteRegKey HKCU "${LEGACYKEY}"
  IfErrors uninstall_failed
  ; Do not recursively remove user data or unknown files.
  RMDir "$INSTDIR"
  Goto uninstall_done
  uninstall_failed:
    SetErrorLevel 1
    Abort "Close Iceon and run Uninstall again."
  uninstall_done:
SectionEnd
