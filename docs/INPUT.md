# Mouse e teclado — input.v1

O controle usa Views nativas no Android e `SendInput` da crate windows no Rust. Não exige novas bibliotecas Android, acessibilidade, root nem permissões de administrador no PC. A conexão conserva o TLS e as credenciais individuais do pareamento.

## Permissão e alvo

Permissão `allowInput` começa falsa no Windows. Ativação da sessão exige outra ação no Android, sempre perdida ao reconectar ou retomar após segundo plano. Use a ação da bandeja para interromper rapidamente. Essa ação invalida a conexão atual para descartar inclusive pedidos de ativação já em trânsito; o Android reconecta desarmado e exige nova ativação. Não existem estados de botão ou tecla presos entre requests.

Texto e teclas vão ao aplicativo atualmente em foco no Windows. Isso é um teclado remoto: um Enter explícito pode confirmar uma ação na janela escolhida. Selecione e confira a janela antes de digitar. Este controle não fornece imagem da tela, seleção de alvo por aplicativo nem modo de arrastar com botão mantido.

Texto tem limite de 1.024 bytes UTF-8 por envio, sem controles; emoji e acentos são convertidos para UTF-16. Enter/Tab usam botões próprios. O conteúdo não é enviado ao clipboard, salvo em histórico nem incluído em erros. Draft no Android é temporário.

## Limitações Windows

SendInput pode ser bloqueado por UIPI: um processo normal não controla automaticamente aplicativos elevados, tela de bloqueio ou desktop protegido do UAC. A API não identifica com segurança qual bloqueio causou a falha. O backend compara a quantidade de eventos injetados com o lote esperado e responde erro sem retry. Não execute o PC Remote como administrador para contornar essas restrições. Jogos, campos especiais e aplicativos podem não aceitar entrada Unicode. Teclas físicas mantidas no PC podem interferir.

Fontes oficiais: [SendInput](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput), [KEYBDINPUT](https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-keybdinput), [PanResponder](https://reactnative.dev/docs/panresponder).

## Validação

Regras de payload, construção de eventos, consentimento, compatibilidade legada e descarte de filas devem ser verificadas pelos testes de ambos os repositórios. A lista executada e pendências estão em [TESTING.md](TESTING.md). Para teste manual, use uma janela descartável de editor de texto: confira movimento, clique, rolagem, acentos/emoji, teclas comuns, bloqueio no PC, segundo plano e reconexão. Não realize testes de digitação em janelas com operações reais do usuário.
