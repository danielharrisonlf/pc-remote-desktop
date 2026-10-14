# Arquitetura 0.1.0

Dois repositórios e builds independentes. A única interface entre os aplicativos é [protocolo 1](PROTOCOL.md), cuja cópia deve ser mantida idêntica em ambos. Não há código compartilhado por caminhos relativos entre repositórios.

## Android

React Native New Architecture, componentes funcionais/hooks e TypeScript estrito. Telas mostram controle, clipboard e configurações. Estado de conexão/sessões e o transporte vivem fora de telas. Slider possui estado local durante arraste, frequência máxima de 10 comandos/s e envio final garantido; confirmações determinam quando snapshots voltam a prevalecer. Capacidades do snapshot governam disponibilidade e explicações dos controles, e o servidor as revalida.

Kotlin integra OkHttp com trust manager específico para o certificado do PC, Android Keystore/AES-GCM em CredentialStore, NsdManager, ClipboardManager, ACTION_SEND, QR e notificações. O cliente usa hostname TLS canônico pc-remote.local, validado normalmente contra o SAN; um Dns exclusivo direciona esse nome ao IPv4 selecionado, mantendo o pin DER. Credencial é lida nativamente para autenticar, sem AsyncStorage. Apenas texto e metadados de UI atravessam o módulo. Observação do clipboard depende de foco real, com baseline novo ao retomar e habilitar. O ciclo de vida interrompe sincronização e impede replay de comandos. Não há garantia de conexão durante suspensão nem timers JS como infraestrutura de segundo plano.

## Windows

Tauri 2 fornece painel React local e bandeja; os comandos IPC configuram o serviço Rust. Rust controla conexão WSS, pareamento em pairing.rs, autorização por dispositivo, DPAPI, mDNS, Core Audio, GSMTC, clipboard e configurações. Fechar a janela oculta; encerrar pela bandeja encerra serviços. Single Instance impede um segundo painel com estado divergente; a inicialização de identidade/serviços ocorre depois dessa proteção.

Core Audio inicializa COM na thread de operação e serializa alterações. Sessões GSMTC vivem numa thread MTA dedicada, com canal de comandos e UUID associado à identidade COM; controles dependem das capacidades reais. Clipboard usa janela nativa invisível com AddClipboardFormatListener/WM_CLIPBOARDUPDATE, owner HWND válido nas escritas e retry limitado quando ocupado. Um watchdog nativo de 5 segundos detecta lacunas de execução, invalida a conexão e exige novo snapshot; ações em espera revalidam a geração/autorização no momento de executar. Não há replay de mídia ou clipboard.

Texto remoto aplicado é reconhecido para impedir eco. Eventos têm UUID/origem, dedup limitada em RAM e revisão ordenada pelo PC. Conflito automático é rejeitado em vez de arbitrado por horário dos aparelhos. Não há outbox ou histórico; sincronização não lê conteúdo existente no primeiro pareamento ou reconexão. As direções precisam estar habilitadas tanto nas preferências locais quanto na conexão.

## Segurança e extensão

Convite de dois minutos combina endereço e identidade do certificado com segredo temporário; sempre há aprovação local no PC. Tokens aleatórios individuais são protegidos no Android e hashes no Windows. Revogação fecha sessões. Mensagens e tipos são validados em runtime com limites antes do dispatch. Uma conexão autenticada pode solicitar apenas ações do enum do protocolo. Sem entrada que represente shell ou comando arbitrário.

Futuras ações devem adicionar um tipo validado, autorização, capacidade anunciada e integração Rust correspondente. Adaptadores de player podem implementar uma interface de mídia, desde que sua capacidade real seja documentada. Transferência de imagens/arquivos exige novo contrato e limites próprios. Fallback de teclado não foi habilitado no MVP: deverá exigir configuração de alvo e checagem da janela antes de enviar teclas.

## Fontes oficiais

- [React Native New Architecture](https://reactnative.dev/architecture/landing-page)
- [Android clipboard em segundo plano](https://developer.android.com/about/versions/10/privacy/changes#clipboard-data)
- [Android clipboard sensível](https://developer.android.com/privacy-and-security/risks/secure-clipboard-handling)
- [OkHttp pinning](https://square.github.io/okhttp/features/https/)
- [Windows Core Audio](https://learn.microsoft.com/en-us/windows/win32/coreaudio/core-audio-apis-in-windows-vista)
- [Windows Media Control](https://learn.microsoft.com/en-us/uwp/api/windows.media.control?view=winrt-26100)
- [Windows clipboard listener](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-addclipboardformatlistener)
- [Windows clipboard exclusion formats](https://learn.microsoft.com/en-us/windows/win32/dataxchg/clipboard-formats)
- [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)
