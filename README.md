# PC Remote Desktop

Controle local do Windows pelo Android, sem conta ou nuvem. Este é o repositório Windows independente; o aplicativo Android está em `pc-remote-android`. Aplicativo 0.5.0, protocolo 1. Consulte [contrato e compatibilidade](docs/PROTOCOL.md), [arquitetura](docs/ARCHITECTURE.md), [mídia](docs/MEDIA-COMPATIBILITY.md), [validação](docs/TESTING.md) e [como publicar uma versão](docs/RELEASE.md).

## Baixar e instalar — Windows 0.5.0

| Aplicativo | Download |
|---|---|
| Windows x64 — instalador | [Baixar instalador .exe](https://github.com/danielharrisonlf/pc-remote-desktop/releases/download/v0.5.0/pc-remote-desktop-0.5.0-x64-setup.exe) |
| Android — APK universal | [Baixar APK 0.4.0](https://github.com/danielharrisonlf/pc-remote-android/releases/download/v0.4.0-kotlin/pc-remote-android-kotlin-0.4.0-evaluation.apk) |

As Releases dos dois repositórios são públicas. As páginas de [Release Windows](https://github.com/danielharrisonlf/pc-remote-desktop/releases/tag/v0.5.0) e [Release Android](https://github.com/danielharrisonlf/pc-remote-android/releases/tag/v0.4.0-kotlin) também incluem os hashes SHA-256.

Instale o `.exe` no Windows e abra o PC Remote pela bandeja. **Quem já usa a 0.4.0 ou anterior instala a 0.5.0 manualmente uma vez**, por cima da instalação atual (mantenha a opção padrão da página do instalador e não marque a exclusão dos dados); pareamentos e configurações são mantidos. Da 0.5.0 em diante as atualizações chegam pelo próprio app (veja [Atualizações](#atualizações)). No celular, abra o APK pelo gerenciador de arquivos e autorize a instalação dessa fonte quando solicitado. Conecte os dois à mesma rede local, clique em **Parear celular** no PC, digite no celular o código de 8 dígitos (ou leia o QR) e confirme o pedido no Windows. Nenhum dos dois precisa de ferramentas de desenvolvimento.

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

O instalador fica em `src-tauri/target/x86_64-pc-windows-msvc/release/bundle/` (ou `target/release/bundle/` sem target explícito). A build assina o instalador para o atualizador e exige `TAURI_SIGNING_PRIVATE_KEY`; sem a chave, [RELEASE.md](docs/RELEASE.md) mostra como gerar um instalador só para teste. Instale o pacote NSIS `.exe` na sessão do usuário. Fechar a janela oculta o painel e mantém os serviços Rust; use **Encerrar** na bandeja para sair. Inicialização com Windows é configurável no painel. As versões exatas estão em `package-lock.json` e `src-tauri/Cargo.lock`; consulte também [VERSIONS.md](docs/VERSIONS.md).

## Parear e usar

1. Conecte os dois aparelhos à mesma LAN. Abra o painel na bandeja e inicie o pareamento.
2. No Android, digite o código de 8 dígitos e escolha o PC encontrado na rede, ou leia o QR. Com o código, o celular mostra uma faixa de cores que deve ser igual à exibida no PC.
3. Confirme o pedido e o nome do celular no Windows. O código expira em 120 segundos.
4. Volume e mídia ficam disponíveis conforme o estado e as capacidades reais. Para remover acesso, revogue o dispositivo no painel.
5. Use **Enviar para o PC** / **Copiar do PC** no Android. Para sincronizar automaticamente, ative as direções desejadas **em ambos os aplicativos**. Elas começam desligadas.

Se a rede bloquear a descoberta (mDNS), use o QR. A descoberta não prova identidade: no pareamento por código, compare as cores antes de permitir. Após reinstalar ou apagar a identidade do PC, remova o pareamento antigo e pareie novamente.

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

1. No Windows, abra **Transferência** (ou **Controles rápidos** na Visão geral) e ligue **Permitir mouse e teclado pelo celular**. A opção começa desligada.
2. No Android, em **Mouse e teclado**, toque em **Ativar controle** nesta conexão.
3. Deslize no touchpad para mover o cursor e toque para clicar. Os botões abaixo oferecem clique direito, duplo clique, rolagem e as teclas comuns. Digite no campo de texto e toque em **Digitar**.
4. O alvo é o aplicativo em foco no Windows. Selecione a janela desejada antes de enviar texto. O controle não exibe a tela do PC.
5. Para interromper, toque em **Desligar** no celular, use **Interromper** em Transferência ou na Visão geral do Windows, ou o menu da bandeja. Bloquear a permissão no PC também interrompe o controle.

A ativação vale para a conexão: trocar de aba no Android não a desliga. Em segundo plano ou sem foco, o Android desarma o PC na hora e reativa sozinho ao voltar, na mesma conexão. Desconectar, **Desligar** ou bloquear no PC exigem ativar novamente; movimentos, cliques e textos pendentes não são repetidos. Texto digitado fica somente em RAM e não entra em logs/histórico. O limite por envio é **1.024 bytes UTF-8**, sem caracteres de controle; use os botões Enter e Tab para essas teclas. Aplicativos elevados, tela de bloqueio/UAC e algumas interfaces podem recusar entrada. Teclas físicas pressionadas no PC podem interferir. Veja [INPUT.md](docs/INPUT.md) para detalhes e validações.

## Imagens e PDFs do celular

1. No Windows, abra **Transferência** e ligue **Receber imagens e PDFs do celular** no cartão **Arquivos do celular**. A opção começa desligada.
2. No Android, escolha os arquivos no aplicativo ou use **Compartilhar** em outro app. São aceitos PNG, JPG e PDF de até 50 MB, um de cada vez.
3. O PC salva em **Downloads\PC Remote**, nunca sobrescreve um arquivo existente (acrescenta `(1)`, `(2)`…) e marca cada arquivo como vindo da internet. Uma notificação do Windows avisa quando o arquivo chega; **Abrir pasta** mostra a pasta.

O envio usa uma conexão separada autorizada por um ticket de uso único da sessão já pareada. Nome, tipo, tamanho e SHA-256 são conferidos antes de salvar; envios interrompidos são descartados. Desligar a opção no PC interrompe o envio em andamento. O painel guarda apenas nome, tamanho e horário dos últimos 12 arquivos, em RAM. Requer PC e Android com a extensão files.v1; versões anteriores continuam funcionando sem ela.

## Atualizações

Da 0.5.0 em diante o PC Remote se atualiza pelo próprio app. Ele lê `latest.json` da release mais recente 10 s depois de abrir e no intervalo escolhido em **Configurações → Atualizações** (1 h, 6 h, 24 h ou **Manual**; padrão 6 h).

1. Com **Baixar atualizações automaticamente** (ligado por padrão), a versão nova é baixada em segundo plano, com barra de progresso. Antes de guardar o instalador, o app confere a assinatura com a chave pública embutida e recusa arquivos adulterados ou de outra versão.
2. O cabeçalho mostra **Atualização pronta** e o cartão **Atualizações** mostra as novidades e o botão **Reiniciar para atualizar**. Nada é instalado antes desse clique. Com o download automático desligado, o botão é **Baixar e instalar**.
3. Ao reiniciar, o PC Remote fecha, o instalador roda em modo passivo (só uma barra de progresso, sem a página "já instalado" e sem UAC) e o app abre de novo. Pareamentos e configurações são mantidos; a conexão com o celular cai durante a instalação.

Se algo falhar, o cartão explica o motivo e oferece **Tentar de novo** e o link da página da versão, para baixar o instalador pelo navegador. A instalação manual da 0.5.0 sobre a 0.4.0 ou anterior é necessária uma única vez, porque essas versões não têm o atualizador. Como publicar uma versão está em [RELEASE.md](docs/RELEASE.md).

## Integração contínua

O modelo reproduzível está em [docs/ci/github-actions.yml](docs/ci/github-actions.yml). Ele executa testes e gera artefatos de avaliação. A autenticação GitHub disponível nesta entrega não possui o escopo `workflow`, portanto o modelo não está ativo. Para ativá-lo, com uma autenticação autorizada para workflows, mova o arquivo para `.github/workflows/build.yml` e publique esse commit. Os builds e testes locais estão registrados em [docs/TESTING.md](docs/TESTING.md).

A revisão de 15/10/2026 está documentada na [auditoria de segurança](docs/SECURITY-AUDIT.md) e no [relatório de bugs e regressões](docs/BUG-AUDIT-2026-10-15.md), com evidências e limites da validação.
