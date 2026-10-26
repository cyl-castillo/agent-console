; Compiles src-tauri/windows/hooks.nsh the way Tauri's installer.nsi would:
; same defines, same includes, the hooks inserted in an install section.
; Run on a Windows runner (makensis ships with the image):
;   makensis /V2 tests/nsis/hooks-smoke.nsi
; It only checks that the hooks assemble; nothing is installed.
Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"
!define MAINBINARYNAME "agent-console"
!define PRODUCTNAME "Agent Console"
!include "..\..\src-tauri\windows\hooks.nsh"

Name "${PRODUCTNAME} hooks smoke"
OutFile "hooks-smoke.exe"
InstallDir "$LOCALAPPDATA\${PRODUCTNAME}"
RequestExecutionLevel user

Section "Install"
  !ifmacrodef NSIS_HOOK_PREINSTALL
    !insertmacro NSIS_HOOK_PREINSTALL
  !endif
  !ifmacrodef NSIS_HOOK_POSTINSTALL
    !insertmacro NSIS_HOOK_POSTINSTALL
  !endif
SectionEnd

Section "Uninstall"
  !ifmacrodef NSIS_HOOK_PREUNINSTALL
    !insertmacro NSIS_HOOK_PREUNINSTALL
  !endif
  !ifmacrodef NSIS_HOOK_POSTUNINSTALL
    !insertmacro NSIS_HOOK_POSTUNINSTALL
  !endif
SectionEnd
