; Runs after files have been installed, including the updater's /UPDATE path.
; The fixed CLI mode runs before Tauri/single-instance initialization and derives
; the application path internally. It requests UAC only when a repair is needed.
!macro NSIS_HOOK_POSTINSTALL
  Push $0
  DetailPrint "Configurando o acesso do PC Remote à rede local..."
  ClearErrors
  ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --configure-network' $0
  ${If} ${Errors}
    StrCpy $0 3
  ${EndIf}

  ${If} $0 == 0
    DetailPrint "Permissões locais de rede verificadas."
  ${ElseIf} $0 == 2
    DetailPrint "A autorização de rede foi cancelada."
    MessageBox MB_OK|MB_ICONINFORMATION "PC Remote foi instalado. A autorização de rede foi cancelada. Para permitir a conexão depois, abra Parear celular e clique em Permitir conexões na rede." /SD IDOK
  ${Else}
    DetailPrint "Não foi possível confirmar as permissões locais de rede."
    MessageBox MB_OK|MB_ICONINFORMATION "PC Remote foi instalado, mas as permissões de rede não puderam ser confirmadas. Abra Parear celular e clique em Permitir conexões na rede para tentar novamente." /SD IDOK
  ${EndIf}
  ClearErrors
  Pop $0
!macroend
