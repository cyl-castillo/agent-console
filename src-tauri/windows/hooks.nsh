; NSIS hooks for the Agent Console installer (wired through
; bundle.windows.nsis.installerHooks in tauri.conf.json).
;
; Why this exists: on update, the Tauri updater launches this installer and
; calls std::process::exit(0) right away. The installer's own check kills a
; still-running agent-console.exe and waits 500 ms — but a process with PTY
; children and a dozen threads can take longer than that to release its
; image, and the next step opens the exe for writing. A user then saw
; "Error opening file for writing: ...\agent-console.exe" (v0.92.x, Windows
; 11). So before any file is copied: finish off anything still holding the
; binaries, then wait until the exe can actually be opened for writing.

!macro NSIS_HOOK_PREINSTALL
  ; Belt and braces: end any instance still around with its children (the
  ; template already asked, or killed, by name; a tree kill also takes the
  ; PTY children that keep the handle alive a little longer). The hook-bridge
  ; sidecar is left alone on purpose: hooks and the MCP server run from the
  ; copy under AppData, so the one in the install dir is never locked.
  nsExec::ExecToLog 'taskkill /F /T /IM "${MAINBINARYNAME}.exe"'
  Pop $0

  ; Up to 20 s, in 500 ms steps, until the previous exe opens for writing
  ; (or is gone — a fresh install has nothing to wait for).
  StrCpy $R9 0
  ac_wait_unlock:
    IfFileExists "$INSTDIR\${MAINBINARYNAME}.exe" 0 ac_unlocked
    ClearErrors
    FileOpen $R8 "$INSTDIR\${MAINBINARYNAME}.exe" a
    IfErrors ac_still_locked
      FileClose $R8
      Goto ac_unlocked
  ac_still_locked:
    IntOp $R9 $R9 + 1
    IntCmp $R9 40 ac_unlocked ac_keep_waiting ac_unlocked
  ac_keep_waiting:
    Sleep 500
    Goto ac_wait_unlock
  ac_unlocked:
!macroend
