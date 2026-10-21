; Keep an existing direct installation in place when the public name changes.
; The legacy registry and directory names are compatibility identifiers.
Var PreelockLegacyInstall
!macro NSIS_HOOK_PREINSTALL
  ReadRegStr $PreelockLegacyInstall HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\VYSTRIQ" "InstallLocation"
  ${If} $PreelockLegacyInstall != ""
    ; Tauri records InstallLocation in quotes.
    StrCpy $R0 $PreelockLegacyInstall 1
    ${If} $R0 == '$\"'
      StrCpy $PreelockLegacyInstall $PreelockLegacyInstall -1 1
    ${EndIf}
    ${If} ${FileExists} "$PreelockLegacyInstall\vystriq.exe"
      StrCpy $INSTDIR $PreelockLegacyInstall
    ${Else}
      StrCpy $PreelockLegacyInstall ""
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ${If} $PreelockLegacyInstall != ""
    ; Remove only known shortcuts whose target belongs to this installation.
    !insertmacro IsShortcutTarget "$SMPROGRAMS\VYSTRIQ.lnk" "$INSTDIR\vystriq.exe"
    Pop $R0
    ${If} $R0 == 1
      Delete "$SMPROGRAMS\VYSTRIQ.lnk"
    ${EndIf}
    !insertmacro IsShortcutTarget "$DESKTOP\VYSTRIQ.lnk" "$INSTDIR\vystriq.exe"
    Pop $R0
    ${If} $R0 == 1
      Delete "$DESKTOP\VYSTRIQ.lnk"
    ${EndIf}
    ; The new installer has already written the replacement uninstall record.
    DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\VYSTRIQ"
  ${EndIf}
!macroend
