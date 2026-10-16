# Arquitetura

Dois repositórios e builds independentes. A única interface entre os aplicativos é [protocolo 1](PROTOCOL.md), cuja cópia deve ser mantida idêntica em ambos. Não há código compartilhado por caminhos relativos entre repositórios.

## Android

Kotlin com Views nativas, sem Compose nem React Native. Quatro abas (Touchpad, Mídia, Transferir, Conexão); estado de conexão/sessões e o transporte vivem num cliente da Application, fora das telas. O slider de volume tem estado local durante o arraste, no máximo 10 comandos/s e envio final garantido; confirmações determinam quando snapshots voltam a prevalecer. Capacidades do snapshot governam a disponibilidade dos controles, e o servidor as revalida.

O cliente integra OkHttp com trust manager específico para o certificado do PC, Android Keystore/AES-GCM em CredentialStore, NsdManager, ClipboardManager, ACTION_SEND, QR e notificações. O cliente usa hostname TLS canônico pc-remote.local, validado normalmente contra o SAN; um Dns exclusivo direciona esse nome ao IPv4 selecionado, mantendo o pin DER. Observação do clipboard depende de foco real, com baseline novo ao retomar e habilitar. O ciclo de vida interrompe sincronização e impede replay de comandos. Não há garantia de conexão durante suspensão.

## Windows

Tauri 2 fornece painel React local e bandeja; os comandos IPC configuram o serviço Rust. Rust controla conexão WSS, pareamento em pairing.rs, autorização por dispositivo, DPAPI, mDNS, Core Audio, GSMTC, clipboard, recebimento de arquivos em files.rs e configurações. Fechar a janela oculta; encerrar pela bandeja encerra serviços. Single Instance impede um segundo painel com estado divergente; a inicialização de identidade/serviços ocorre depois dessa proteção.

Core Audio inicializa COM na thread de operação e serializa alterações. Sessões GSMTC vivem numa thread MTA dedicada, com canal de comandos e UUID associado à identidade COM; controles dependem das capacidades reais. Clipboard usa janela nativa invisível com AddClipboardFormatListener/WM_CLIPBOARDUPDATE, owner HWND válido nas escritas e retry limitado quando ocupado. Um watchdog nativo de 5 segundos detecta lacunas de execução, invalida a conexão e exige novo snapshot; ações em espera revalidam a geração/autorização no momento de executar. Não há replay de mídia ou clipboard.

Texto remoto aplicado é reconhecido para impedir eco. Eventos têm UUID/origem, dedup limitada em RAM e revisão ordenada pelo PC. Conflito automático é rejeitado em vez de arbitrado por horário dos aparelhos. Não há outbox ou histórico; sincronização não lê conteúdo existente no primeiro pareamento ou reconexão. As direções precisam estar habilitadas tanto nas preferências locais quanto na conexão.

## Segurança e extensão

Convite de dois minutos combina endereço e identidade do certificado com segredo temporário; sempre há aprovação local no PC. Tokens aleatórios individuais são protegidos no Android e hashes no Windows. Revogação fecha sessões. Mensagens e tipos são validados em runtime com limites antes do dispatch. Uma conexão autenticada pode solicitar apenas ações do enum do protocolo. Sem entrada que represente shell ou comando arbitrário.

Futuras ações devem adicionar um tipo validado, autorização, capacidade anunciada e integração Rust correspondente. Adaptadores de player podem implementar uma interface de mídia, desde que sua capacidade real seja documentada. Imagens e PDFs do celular usam o contrato próprio files.v1 (ver abaixo); outros tipos exigiriam nova extensão. Fallback de teclado não foi habilitado no MVP: deverá exigir configuração de alvo e checagem da janela antes de enviar teclas.

## Arquivos do celular (files.v1)

`files.rs` concentra tickets, sanitização de nomes, regras de MIME/extensão e assinatura, escrita da parte, marca de origem e nome final. O fluxo:

1. O celular envia `file.offer` na conexão de controle. `connection.rs` confere negociação e `allowFiles`, valida os campos e chama `Core::file_offer` numa thread bloqueante, que prepara `Downloads\PC Remote`, confere espaço livre e grava o ticket no `Runtime` (um único slot por PC). A resposta é `file.ticket` e depois `ack`.
2. O celular abre `/v1/files` no mesmo listener. O handshake roteia pelo caminho; o socket é reembrulhado com limites próprios (binário 64 KiB, texto 4 KiB). `file.begin` consome o ticket e transforma o slot em transferência ativa.
3. Cada frame binário passa por `Core::file_check` (sessão, epoch, `allowFiles`, slot) e entra numa fila limitada a 16 blocos. Uma tarefa `spawn_blocking` calcula o SHA-256 e grava com buffer de 256 KiB, sem bloquear o runtime; a fila cheia aplica contrapressão no socket.
4. Em `file.end`, o PC confere tamanho, hash e assinatura, faz `sync_all`, grava `Zone.Identifier` e move com `MoveFileExW` sem substituição. Depois registra o metadado (até 12 itens em RAM), chama o notificador injetado e responde `file.done`.

Cancelamento, `allowFiles` desligado, fim da sessão ou revogação limpam o slot e acordam a transferência, que apaga a parte. Um guarda (`TransferGuard`) também apaga a parte e libera o slot em qualquer saída do handler. A notificação usa `tauri-plugin-notification`; nos testes o notificador não é configurado ou captura as chamadas, e a pasta é temporária.

## Atualizações do Windows (0.5.0)

`update.rs` usa `tauri-plugin-updater` a partir de três comandos do app; a WebView não recebe nenhuma permissão `updater:*`.

1. `update_check` monta o atualizador com o endpoint de `tauri.conf.json` (`releases/latest/download/latest.json`; em debug, `PCREMOTE_UPDATE_ENDPOINT` o substitui), tempo limite de 30 s, e guarda o `Update` encontrado em `UpdateState`. O plugin só devolve versões maiores que a em execução. Retorna versão, notas, data e se o instalador dessa versão já está baixado.
2. `update_download` baixa o `Update` guardado e envia o progresso por um `Channel` (`started`, `progress` a cada 64 KiB, `finished`). O plugin verifica a assinatura minisign e a versão assinada antes de devolver os bytes, que ficam em memória no Rust. Um segundo download simultâneo é recusado; uma checagem nova que encontra outra versão descarta o download anterior.
3. `update_install` confere de novo que o instalador baixado é de uma versão maior, grava-o em `%TEMP%` e roda `setup.exe /P /UPDATE /R`. Antes, `on_before_exit` limpa o ícone da bandeja; depois o processo sai com `exit(0)`, o que libera o executável, a porta 43127, o mDNS e a trava do Single Instance. `prevent_close` não interfere, porque não há evento de janela. O NSIS fecha pelo Restart Manager uma instância remanescente, instala e reabre o app.

rustls passou a ter aws-lc-rs e ring compilados (o plugin habilita ring); `connection::install_crypto_provider` fixa aws-lc-rs como provedor do processo no início de `run()`, antes do servidor e das requisições do atualizador, que validam o certificado do GitHub pela raiz do Windows (`rustls-platform-verifier`).

No painel, `lib/updates.ts` é a máquina de estados pura (verificando → disponível → baixando → verificando assinatura → pronta → instalando, ou erro com a etapa a repetir), testada no Vitest; `hooks/useUpdater.ts` agenda as verificações, dispara o download automático e chama os comandos. `components/UpdatePanel.tsx` (progresso, erro com link da release, notas) é usado no cartão **Atualizações** e no diálogo aberto pelo cabeçalho.

Os testes do fluxo ficam em `src-tauri/tests/updater.rs`: um app `MockRuntime` configurado como o `tauri.conf.json` contra um servidor HTTP local, com fixtures assinadas pela CLI do Tauri. Por usar o app simulado (que importa funções do Common Controls v6), o `build.rs` embute um manifesto só nos executáveis de teste (`rustc-link-arg-tests`); o executável de release mantém o manifesto gerado pelo tauri-build. Publicação e chave: [RELEASE.md](RELEASE.md).

## Fontes oficiais

- [Android clipboard em segundo plano](https://developer.android.com/about/versions/10/privacy/changes#clipboard-data)
- [Android clipboard sensível](https://developer.android.com/privacy-and-security/risks/secure-clipboard-handling)
- [OkHttp pinning](https://square.github.io/okhttp/features/https/)
- [Windows Core Audio](https://learn.microsoft.com/en-us/windows/win32/coreaudio/core-audio-apis-in-windows-vista)
- [Windows Media Control](https://learn.microsoft.com/en-us/uwp/api/windows.media.control?view=winrt-26100)
- [Windows clipboard listener](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-addclipboardformatlistener)
- [Windows clipboard exclusion formats](https://learn.microsoft.com/en-us/windows/win32/dataxchg/clipboard-formats)
- [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)

## Interface

A impressão digital SHA-256 é mostrada em 16 blocos de 4 caracteres e numa faixa de 8 cores derivada por XOR dos 32 bytes. O mesmo algoritmo existe em `src/lib/fingerprint.ts` (Windows) e `Fingerprint.kt` (Android), com valores de teste idênticos. No pareamento por código, o celular fixa o certificado da primeira conexão e mostra essa faixa para comparação com o PC. No Windows a faixa aparece só na gaveta de pareamento; Visão geral mostra o código curto e Diagnóstico os 16 blocos.

Windows: `App.tsx` só compõe o shell. `hooks/` concentra estado (`useStatus` com polling encadeado sem sobreposição e último estado preservado em falhas, `useSettings` com alterações serializadas, `useTheme`, `useUpdater`, `useToasts`), `components/` os blocos visuais (barra lateral retrátil, cabeçalho com status, gaveta de pareamento com aprovação inline, diálogos nativos `<dialog>`, toasts em top layer) e `views/` as páginas Visão geral, Dispositivos, Transferência, Diagnóstico e Configurações. O backend expõe também horário do último contato por dispositivo, horário dos pedidos de pareamento, eventos com data e o histórico de transferências apenas com metadados, tudo em RAM.
