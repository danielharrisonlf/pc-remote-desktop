# Versões verificadas em 24/10/2026

Os lockfiles são a referência exata de todas as dependências transitivas. As versões abaixo foram usadas nas builds locais que acompanham esta entrega; não são versões beta.

| Componente | Versão |
|---|---|
| Desktop / Android / protocolo | 0.6.6 / 0.6.8-kotlin / 1 + extensões opcionais input.v1, files.v1, files.v2, search.v1, info.v1, power.v1, clipboard.v2, wol.v1, numpad.v1, presentation.v1 e deck.v1 |
| Node / npm | 24.21.0 / 11.19.0 |
| Rust / MSVC / Windows SDK de build | 1.97.1 / 19.44 / 10.0.26100.0 |
| Tauri Rust / CLI / API JS | 2.12.3 / 2.12.2 / 2.12.3 |
| Tauri Single Instance | 2.5.3 |
| Tauri Notification (notify-rust) | 2.5.1 (4.18.2) |
| Tauri Updater (minisign-verify, reqwest) | 2.13.3 (0.3.0, 0.13.5); features `rustls-tls` e `system-proxy`, sem `zip` |
| semver | 1.0.28 |
| image (PNG/JPEG/BMP puro Rust) | 0.25.10 |
| windows / windows-registry (`cfg(windows)`) | 0.62.2 / 0.6.1 |
| arboard / zbus / x11rb / libc (`cfg(target_os = "linux")`) | 3.6.1 / 5.14.0 / 0.13.2 / 0.2.177 |
| Desktop React / TypeScript / Vite | 19.3.0 / 5.9.3 / 8.3.4 |
| Vitest | 5.0.4 |
| Android: Kotlin integrado ao AGP / Android Gradle Plugin / Gradle | 2.2.10 / 9.2.1 / 9.4.1 |
| Android: JDK | Temurin 17.0.20.1 |
| Android: compile / target / mínimo | 37 / 36 / 24 |
| Android: pacote SDK / Build Tools | android-37.0 / 37.0.0 |
| Android: OkHttp / Google Play Services Code Scanner | 4.12.0 / 16.1.0 |

O app Android é Kotlin com Views nativas, sem React Native, Hermes, Compose ou NDK. A build de release não depende de servidor de desenvolvimento.

Target Android 36: o acesso LAN usa INTERNET e NSD. Ao elevar target para 37, implementar ACCESS_LOCAL_NETWORK com solicitação contextual ou o seletor autorizado do sistema, conforme [documentação Android](https://developer.android.com/privacy-and-security/local-network-permission). Não usar serviço em primeiro plano como alternativa à restrição de clipboard.

## Compatibilidade entre versões

| Android | Desktop | Compatibilidade |
|---|---|---|
| 0.6.6-kotlin | 0.6.5 | Mesmo protocolo e extensões da 0.6.4; nova identidade visual clean moderna (Precision Dual Prism em fundo porcelana clara), ícone adaptativo e instalador atualizados |
| 0.6.3–0.6.5-kotlin | 0.6.4 | Mesmo protocolo e extensões da 0.6.2/0.6.3; identidade vetorial unificada, ícones adaptativos e correções de clipboard de imagens e entrega XTEST no Linux |
| 0.6.2-kotlin | 0.6.3 | Protocolo 1 + input.v1, files.v1, files.v2, search.v1, info.v1, power.v1, clipboard.v2, wol.v1, numpad.v1, presentation.v1 e deck.v1: tudo da 0.6.2, mais Wake-on-LAN (MAC broadcast), teclado numérico dedicado (Keypad), controle de slides/apresentação (Presentation), deck de 12 atalhos do sistema (Deck) e latência de mouse minimizada com TCP_NODELAY e 60 FPS |
| 0.6.1-kotlin | 0.6.1 e 0.6.2 | Protocolo 1 + input.v1, files.v1, files.v2, search.v1, info.v1, power.v1 e clipboard.v2: tudo da 0.6.0, mais bloquear, suspender, desligar e reiniciar o PC (agora ou agendado) e imagens na área de transferência nos dois sentidos (Windows e Linux/Ubuntu na 0.6.2) |
| 0.6.1-kotlin | 0.6.0 | O PC ignora power.v1 e clipboard.v2: sem energia do PC e clipboard só com texto; vários PCs e controles de mídia fora do app funcionam |
| 0.6.0-kotlin | 0.6.1 | Formato 0.6.0, sem campos novos |
| 0.6.0-kotlin | 0.6.0 | Protocolo 1 + input.v1, files.v1, files.v2, search.v1 e info.v1: arquivos de qualquer tipo até 1 GB nos dois sentidos, touchpad e clipboard prontos após o pareamento, pesquisa no YouTube e nome do Windows |
| 0.6.0-kotlin | 0.4.0 e 0.5.0 | O PC ignora files.v2, search.v1 e info.v1: PNG/JPG/PDF até 50 MB só do celular para o PC, sem pesquisa; touchpad pede **Ativar touchpad** e o PC ainda pede as chaves de mouse/teclado, arquivos e direções do clipboard |
| 0.4.0 e 0.5.0-kotlin | 0.6.0 | Formato anterior; arquivos e mouse/teclado já liberados no PC; o clipboard sincroniza nas direções ligadas no celular |
| 0.4.0-kotlin | 0.5.0 e 0.4.0 | Protocolo 1; input.v1 após consentimento nos dois aplicativos; files.v1 com **Receber imagens e PDFs do celular** ligado no PC. A 0.5.0 só acrescenta a atualização pelo app no Windows, sem mudança de protocolo |
| 0.4.0-kotlin | 0.2.x e 0.3.0 | input.v1, volume (também pelas teclas físicas), mídia e clipboard; Arquivos pede para atualizar o PC e nenhum pedido de arquivo é enviado |
| 0.2.0–0.3.0-kotlin | 0.4.0 e 0.5.0 | Contrato anterior preservado; o PC não anuncia `file.transfer` nem envia o estado `files` |
| 0.2.x–0.4.0-kotlin | 0.1.x | Volume, mídia e clipboard; sem mouse/teclado |
| Qualquer implementação | Sem protocolo 1 | Incompatível; rejeitado |

A matriz descreve contrato; capacidades do dispositivo/player definem os controles reais. Não equivale a certificação de todos os players/hardwares.
