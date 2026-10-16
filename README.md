# PC Remote Desktop

Controle local do Windows pelo Android, sem conta ou nuvem. Este é o repositório Windows independente; o aplicativo Android está em `pc-remote-android`. Aplicativo 0.2.2, protocolo 1. Consulte [contrato e compatibilidade](docs/PROTOCOL.md), [arquitetura](docs/ARCHITECTURE.md), [mídia](docs/MEDIA-COMPATIBILITY.md) e [validação](docs/TESTING.md).

## Baixar e instalar — Windows 0.2.2

| Aplicativo | Download |
|---|---|
| Windows x64 — instalador | [Baixar instalador .exe](https://github.com/danielharrisonlf/pc-remote-desktop/releases/download/v0.2.2/pc-remote-desktop-0.2.2-x64-setup.exe) |
| Android Kotlin — APK universal, branch native/kotlin | [Baixar APK Kotlin 0.2.4](https://github.com/danielharrisonlf/pc-remote-android/releases/download/v0.2.4-kotlin/pc-remote-android-kotlin-0.2.4-evaluation.apk) |
| Android ARM64 — celular compatível | [Baixar APK](https://github.com/danielharrisonlf/pc-remote-android/releases/download/v0.2.1/pc-remote-android-0.2.1-arm64-evaluation.apk) |
| Android x86_64 — emulador | [Baixar APK para emulador](https://github.com/danielharrisonlf/pc-remote-android/releases/download/v0.2.1/pc-remote-android-0.2.1-x86_64-evaluation.apk) |

Os downloads estão nas Releases privadas: entre no GitHub com uma conta que tenha acesso aos dois repositórios. As páginas de [Release Windows](https://github.com/danielharrisonlf/pc-remote-desktop/releases/tag/v0.2.2) e [Release Android](https://github.com/danielharrisonlf/pc-remote-android/releases/tag/v0.2.4-kotlin) também incluem os hashes SHA-256.

Instale o `.exe` no Windows e abra o PC Remote pela bandeja. No celular, abra o APK ARM64 pelo gerenciador de arquivos e autorize a instalação dessa fonte quando solicitado. Conecte ambos à mesma rede local, inicie o pareamento no PC, escaneie o QR ou informe os dados exibidos e confirme o pedido no Windows. Os aplicativos instalados não precisam de Metro, Expo Go ou ferramentas de desenvolvimento.

Esta é uma versão de avaliação: APK assinado com chave debug e instalador sem Authenticode. A validação em celular físico e players reais permanece pendente; consulte [testes e limitações](docs/TESTING.md).

A versão 0.2.1 prioriza no QR o endereço IPv4 de uma interface com rota padrão, evitando escolher primeiro um adaptador virtual sem rota padrão. Mantém os demais endereços e o preenchimento manual. VPNs ou múltiplas rotas padrão ainda podem exigir informar o IPv4 alcançável pelo celular; a seleção não garante conectividade com todas as redes.

## Desenvolvimento e instalação

Windows 10 1809+ ou Windows 11, WebView2 Runtime, Node LTS >=22.11, Rust estável **MSVC x64**, Visual Studio Build Tools com Desktop Development with C++ e Windows SDK. O aplicativo usa a sessão do usuário; não precisa ser iniciado como administrador. MinGW não é o toolchain suportado para distribuir Tauri.

```powershell
git clone https://github.com/danielharrisonlf/pc-remote-desktop.git
cd pc-remote-desktop
npm ci
rustup target add x86_64-pc-windows-msvc
npm run tauri dev
```

Se o toolchain padrão for GNU, execute os comandos Rust com `cargo +stable-x86_64-pc-windows-msvc` ou configure o toolchain por projeto. Não é necessário modificar o padrão global.

```powershell
npm run typecheck
npm test
npm run build
cargo +stable-x86_64-pc-windows-msvc test --manifest-path src-tauri/Cargo.toml
npm run tauri build -- --target x86_64-pc-windows-msvc
```

O instalador fica em `src-tauri/target/x86_64-pc-windows-msvc/release/bundle/` (ou `target/release/bundle/` sem target explícito). Instale o pacote NSIS `.exe` na sessão do usuário. Fechar a janela oculta o painel e mantém os serviços Rust; use **Encerrar** na bandeja para sair. Inicialização com Windows é configurável no painel. As versões exatas estão em `package-lock.json` e `src-tauri/Cargo.lock`; consulte também [VERSIONS.md](docs/VERSIONS.md).

## Parear e usar

1. Conecte os dois aparelhos à mesma LAN. Abra o painel na bandeja e inicie o pareamento.
2. No Android, escaneie o QR ou informe endereço do PC, porta 43127, código temporário e fingerprint completo mostrado pelo PC. Confira o endereço escolhido se houver VPN ou várias interfaces.
3. Confirme o pedido e o nome do celular no Windows. O código expira em 120 segundos.
4. Volume e mídia ficam disponíveis conforme o estado e as capacidades reais. Para remover acesso, revogue o dispositivo no painel.
5. Use **Enviar para o PC** / **Copiar do PC** no Android. Para sincronizar automaticamente, ative as direções desejadas **em ambos os aplicativos**. Elas começam desligadas.

Se a rede bloquear descoberta, informe o IPv4 do PC manualmente. A descoberta não é prova de identidade. Não copie fingerprint de uma fonte desconhecida. Após reinstalar ou apagar a identidade do PC, remova o pareamento antigo e pareie novamente.

## Firewall

Permita TCP 43127 de entrada para o executável somente no perfil **Privado** e na sub-rede local. Descoberta mDNS usa UDP 5353. Rede de convidados, isolamento de clientes Wi-Fi e algumas VPNs impedem conexão/descoberta. O aplicativo não modifica o roteador nem abre portas na internet. Caso o Windows apresente diálogo de firewall, autorize apenas a rede privada. Uma regra explícita criada pelo administrador pode usar o caminho real instalado:

```powershell
# Opcional: PowerShell administrativo, ajuste o caminho para o executável instalado.
New-NetFirewallRule -DisplayName 'PC Remote LAN' -Direction Inbound -Action Allow -Protocol TCP -LocalPort 43127 -Profile Private -RemoteAddress LocalSubnet -Program 'C:\caminho\pc-remote-desktop.exe'
```

## Privacidade e assinatura

Credenciais e chave TLS são protegidas com DPAPI do usuário. Clipboard só texto até 64 KiB UTF-8, sem histórico persistente e sem conteúdo em diagnóstico. A sincronização observa flags de exclusão do Windows, mas não identifica todo conteúdo sensível. A atualização do clipboard nunca cola, executa comandos ou abre links. No MVP há um controlador conectado por PC.

Para distribuição pública, assine executável e instalador com certificado Authenticode e timestamp do provedor usando `signtool`. Configure `bundle.windows.certificateThumbprint`/`timestampUrl` localmente ou variáveis de CI seguras. Não versione PFX, senhas, credenciais ou chaves. Um instalador sem assinatura pode disparar SmartScreen. Assinatura Authenticode é independente da identidade TLS gerada por instalação.

Veja [testes e pendências](docs/TESTING.md) antes de considerar a build validada em hardware. Os pontos de extensão aceitam apenas ações tipadas e explicitamente autorizadas; não existe endpoint de shell.

## Touchpad e teclado remoto

1. No Windows, abra **Preferências → Mouse e teclado** e habilite **Permitir mouse e teclado pelo celular**. A opção começa desligada.
2. No Android, abra **Touchpad e teclado** e toque em **Ativar controle** nesta conexão.
3. Arraste no touchpad para mover o cursor; toque para clicar. Botões separados oferecem clique direito, clique duplo e rolagem. Digite no campo de texto e envie explicitamente, ou use as teclas comuns.
4. O alvo é o aplicativo em foco no Windows. Selecione a janela desejada antes de enviar texto. O controle não exibe a tela do PC.
5. Para interromper, desative no celular, na Visão geral do Windows ou no menu da bandeja. Bloquear a permissão no PC também interrompe o controle.

A ativação é descartada ao desconectar, suspender ou colocar o Android em segundo plano. É necessário ativar novamente; movimentos, cliques e textos pendentes não são repetidos. Texto digitado fica somente em RAM e não entra em logs/histórico. O limite por envio é **1.024 bytes UTF-8**, sem caracteres de controle; use os botões Enter e Tab para essas teclas. Aplicativos elevados, tela de bloqueio/UAC e algumas interfaces podem recusar entrada. Teclas físicas pressionadas no PC podem interferir. Veja [INPUT.md](docs/INPUT.md) para detalhes e validações.

## Integração contínua

O modelo reproduzível está em [docs/ci/github-actions.yml](docs/ci/github-actions.yml). Ele executa testes e gera artefatos de avaliação. A autenticação GitHub disponível nesta entrega não possui o escopo `workflow`, portanto o modelo não está ativo. Para ativá-lo, com uma autenticação autorizada para workflows, mova o arquivo para `.github/workflows/build.yml` e publique esse commit. Os builds e testes locais estão registrados em [docs/TESTING.md](docs/TESTING.md).

A revisão de 15/10/2026 está documentada na [auditoria de segurança](docs/SECURITY-AUDIT.md) e no [relatório de bugs e regressões](docs/BUG-AUDIT-2026-10-15.md), com evidências e limites da validação.
