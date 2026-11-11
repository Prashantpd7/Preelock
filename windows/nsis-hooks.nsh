; Keep an existing direct installation in place when the public name changes.
; The legacy registry and directory names are compatibility identifiers.
Var PreelockLegacyInstall
Var PreelockLegacyRun
!macro NSIS_HOOK_PREINSTALL
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
!macroend

; M7: uninstall must not leave startup registrations behind. The
; app writes both Run values itself at runtime (the "Preelock"
; open-at-login value and the "Preelock Background" tracker value),
; and Tauri's NSIS uninstaller only removes what the installer
; wrote — so without this hook an uninstall left Windows trying to
; launch a deleted exe at every sign-in. DeleteRegValue is a no-op
; when a value is already absent. User data is untouched: the
; database, pairing token and settings live under AppData and the
; uninstaller never removes them (see scripts/smoke-windows.ps1).
!macro NSIS_HOOK_PREUNINSTALL
  nsExec::Exec 'taskkill /F /IM preelock.exe /T'
  Pop $0
  Sleep 300
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Preelock"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Preelock Background"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "VYSTRIQ"
!macroend
