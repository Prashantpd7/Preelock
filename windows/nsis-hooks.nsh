; Keep an existing direct installation in place when the public name changes.
; The legacy registry and directory names are compatibility identifiers.
Var PreelockLegacyInstall
Var PreelockLegacyRun
!macro NSIS_HOOK_PREINSTALL
  nsExec::Exec 'schtasks.exe /End /TN "PreelockBackground"'
  Pop $0
  nsExec::Exec 'taskkill /F /IM preelock.exe /T'
  Pop $0
  Sleep 300
  ReadRegStr $PreelockLegacyRun HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "VYSTRIQ"
  ${If} $PreelockLegacyRun != ""
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Preelock" "$PreelockLegacyRun"
  ${EndIf}
  ReadRegStr $PreelockLegacyInstall HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\VYSTRIQ" "InstallLocation"
  ${If} $PreelockLegacyInstall != ""
    ; Tauri records InstallLocation in quotes.
    StrCpy $R0 $PreelockLegacyInstall 1
    ${If} $R0 == '$\"'
      StrCpy $PreelockLegacyInstall $PreelockLegacyInstall -1 1
    ${EndIf}
    ${If} ${FileExists} "$PreelockLegacyInstall\preelock.exe"
    ${OrIf} ${FileExists} "$PreelockLegacyInstall\vystriq.exe"
      StrCpy $INSTDIR $PreelockLegacyInstall
    ${Else}
      StrCpy $PreelockLegacyInstall ""
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ${If} $PreelockLegacyInstall != ""
    ; Remove only known shortcuts whose target belongs to this installation.
    !insertmacro IsShortcutTarget "$SMPROGRAMS\VYSTRIQ.lnk" "$INSTDIR\preelock.exe"
    Pop $R0
    ${If} $R0 == 1
      Delete "$SMPROGRAMS\VYSTRIQ.lnk"
    ${EndIf}
    !insertmacro IsShortcutTarget "$DESKTOP\VYSTRIQ.lnk" "$INSTDIR\preelock.exe"
    Pop $R0
    ${If} $R0 == 1
      Delete "$DESKTOP\VYSTRIQ.lnk"
    ${EndIf}
    ; The new installer has already written the replacement uninstall record.
    DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\VYSTRIQ"
  ${EndIf}
  ; Start or resume the independent background supervisor immediately so any
  ; active session or recurring schedules continue across updates/installs.
  ${If} ${FileExists} "$INSTDIR\preelock.exe"
    Exec '"$INSTDIR\preelock.exe" --background'
  ${EndIf}
!macroend

; M7: uninstall must refuse while a strict focus session is active, and when
; permitted must not leave startup registrations or scheduled tasks behind.
; The app writes Run values and the PreelockBackground Scheduled Task at
; runtime; user data under AppData (database, lifecycle checkpoint, pairing
; token, settings) is intentionally untouched (see scripts/smoke-windows.ps1).
!macro NSIS_HOOK_PREUNINSTALL
  ${If} ${FileExists} "$INSTDIR\preelock.exe"
    nsExec::ExecToStack '"$INSTDIR\preelock.exe" --check-uninstall'
    Pop $0
    Pop $1
    ${If} $0 == 2
      MessageBox MB_ICONSTOP|MB_OK "Preelock cannot be uninstalled while a strict focus session is active. Complete the focus session before uninstalling." /SD IDOK
      Abort
    ${EndIf}
  ${EndIf}
  nsExec::Exec 'schtasks.exe /End /TN "PreelockBackground"'
  Pop $0
  nsExec::Exec 'taskkill /F /IM preelock.exe /T'
  Pop $0
  Sleep 300
  nsExec::Exec 'schtasks.exe /Delete /TN "PreelockBackground" /F'
  Pop $0
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Preelock"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Preelock Background"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "VYSTRIQ"
!macroend
