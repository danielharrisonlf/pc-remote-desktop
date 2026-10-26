# PC Remote Desktop

Controle local do Windows e Linux (Ubuntu) pelo Android, sem conta ou nuvem. Este é o repositório desktop independente; o aplicativo Android está em `pc-remote-android`. Aplicativo 0.6.9, protocolo 1. Consulte [contrato e compatibilidade](docs/PROTOCOL.md), [arquitetura](docs/ARCHITECTURE.md), [mídia](docs/MEDIA-COMPATIBILITY.md), [validação](docs/TESTING.md) e [como publicar uma versão](docs/RELEASE.md).

## Baixar e instalar — Windows 0.6.9

| Aplicativo | Download |
|---|---|
| Windows x64 — instalador | [Baixar instalador .exe](https://github.com/danielharrisonlf/pc-remote-desktop/releases/download/v0.6.9/pc-remote-desktop-0.6.9-x64-setup.exe) |
| Android — APK universal | [Baixar APK 0.6.10](https://github.com/danielharrisonlf/pc-remote-android/releases/download/v0.6.10-kotlin/pc-remote-android-kotlin-0.6.10-evaluation.apk) |

As Releases dos dois repositórios são públicas. As páginas de [Release Windows](https://github.com/danielharrisonlf/pc-remote-desktop/releases/tag/v0.6.9) e [Release Android](https://github.com/danielharrisonlf/pc-remote-android/releases/tag/v0.6.10-kotlin) também incluem os hashes SHA-256.

Instale o `.exe` no Windows e abra o PC Remote pela bandeja. **Quem usa a 0.5.0, 0.6.0, 0.6.1, 0.6.2, 0.6.3, 0.6.4, 0.6.5, 0.6.6, 0.6.7 ou 0.6.8 recebe a 0.6.9 pelo próprio app** (veja [Atualizações](#atualizações)); quem usa a 0.4.0 ou anterior instala manualmente uma vez, por cima da instalação atual (mantenha a opção padrão da página do instalador e não marque a exclusão dos dados). Pareamentos e configurações são mantidos. No celular, abra o APK pelo gerenciador de arquivos e autorize a instalação dessa fonte quando solicitado. Conecte os dois à mesma rede local, clique em **Parear celular** no PC, digite no celular o código de 8 dígitos (ou leia o QR) e confirme o pedido no Windows. Nenhum dos dois precisa de ferramentas de desenvolvimento.

Esta é uma versão de avaliação: APK assinado com chave debug e instalador sem Authenticode. A validação em celular físico e players reais permanece pendente; consulte [testes e limitações](docs/TESTING.md).

Correção **0.6.9**: melhora a continuidade das conexões, o limite de tentativas por código e o reparo de permissões em pastas com apóstrofos. Inclui correções no clipboard, na gravação da identidade e nas transferências. Veja as [notas da versão](docs/releases/0.6.9.md).

Atualização **0.6.8**: a instalação e a atualização solicitam ao Windows a configuração das permissões de rede do PC Remote. O pedido de administrador aparece quando necessário; cancelar mantém o app instalado e permite tentar novamente em **Parear celular → Permitir conexões na rede**. Veja as [notas da versão](docs/releases/0.6.8.md).

Atualização **0.6.7**: verificação e reparo de permissões de rede no Firewall do Windows diretamente pelo painel e pela gaveta de pareamento, e inclusão de permissões no relatório de diagnóstico. Use também o Android 0.6.9-kotlin. Veja as [notas da versão](docs/releases/0.6.7.md).

Correção **0.6.6**: distingue código incorreto de convite expirado, conserva o prazo dos pedidos já recebidos e aguarda a conexão do celular para confirmar o pareamento. Use também o Android 0.6.8-kotlin. Veja as [notas da versão](docs/releases/0.6.6.md).

A versão 0.2.1 prioriza no QR o endereço IPv4 de uma interface com rota padrão, evitando escolher primeiro um adaptador virtual sem rota padrão. Mantém os demais endereços e o preenchimento manual. VPNs ou múltiplas rotas padrão ainda podem exigir informar o IPv4 alcançável pelo celular; a seleção não garante conectividade com todas as redes.

## Instalar no Ubuntu 22.04, 24.04 e 26.04 LTS

Use o pacote `.deb`, sem compilar nem instalar ferramentas de desenvolvimento:

```bash
sudo apt install --no-install-recommends ~/Downloads/pc-remote-desktop-0.6.4-amd64.deb
```

Baixe o [.deb amd64](https://github.com/danielharrisonlf/pc-remote-desktop/releases/download/v0.6.4-ubuntu.1/pc-remote-desktop-0.6.4-amd64.deb) ou o [instalador compactado](https://github.com/danielharrisonlf/pc-remote-desktop/releases/download/v0.6.4-ubuntu.1/pc-remote-ubuntu-0.6.4-amd64.tar.gz) da [pré-release Ubuntu](https://github.com/danielharrisonlf/pc-remote-desktop/releases/tag/v0.6.4-ubuntu.1). O pacote instala as dependências e adiciona **PC Remote** ao menu. O instalador compactado também permite extrair e executar `bash install-ubuntu.sh`. Instalação e abertura foram verificadas nas três versões LTS; a validação em um desktop GNOME Wayland real permanece pendente.

No Wayland, inclusive no Ubuntu Desktop 26.04, abra **Configurações → Controle no Ubuntu → Autorizar controle** e permita mouse e teclado na janela do sistema. Para atualizar, encerre o aplicativo e instale o novo `.deb`; os pareamentos são mantidos. Veja [o guia do Ubuntu](docs/UBUNTU.md) para instalação, autorização e geração dos pacotes.

## Desenvolvimento e instalação

Windows 10 1809+ ou Windows 11, WebView2 Runtime, Node LTS >=22.12, Rust estável **MSVC x64**, Visual Studio Build Tools com Desktop Development with C++ e Windows SDK. O aplicativo usa a sessão do usuário; não precisa ser iniciado como administrador. MinGW não é o toolchain suportado para distribuir Tauri.

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

### Compilação e empacotamento no Linux (Ubuntu 22.04 / 24.04 / 26.04 LTS)

Instale os pacotes de sistema necessários para compilar o Tauri v2 e os backends de entrada, bandeja e WebKitGTK no Ubuntu / Debian:

```bash
sudo apt-get update && sudo apt-get install -y \
  libwebkit2gtk-4.1-dev \
  build-essential \
  pkg-config \
  curl \
  wget \
  file \
  libxdo-dev \
  libssl-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev
```

Com Node e Rust instalados, gere o `.deb` e o instalador compactado. Para distribuição nas três versões LTS, compile no Ubuntu 22.04:

```bash
npm run build:ubuntu
```

O `.deb`, o script de instalação, instruções e hashes ficam em `release-assets/<versão>/ubuntu-amd64/`; o `.tar.gz` fica em `release-assets/<versão>/`. Não é necessária a chave de assinatura do atualizador Windows. No Linux, o início automático cria/remove `~/.config/autostart/pc-remote-desktop.desktop` seguindo o padrão XDG.

## Parear e usar

1. Conecte os dois aparelhos à mesma LAN. Abra o painel na bandeja e inicie o pareamento.
2. No Android, digite o código de 8 dígitos e escolha o PC encontrado na rede, ou leia o QR. Com o código, o celular mostra uma faixa de cores que deve ser igual à exibida no PC.
3. Confirme o pedido, as cores de identidade e o nome do celular no Windows. O código aceita novos pedidos por 120 segundos; depois de recebido, o pedido tem mais 120 segundos para aprovação.
4. Pronto: touchpad, volume, mídia, arquivos e área de transferência funcionam sem ligar nada. Volume e mídia seguem as capacidades reais de cada player; o celular controla primeiro o player que o Windows trata como atual e pode abrir pesquisas do YouTube no navegador padrão do PC. Para remover acesso, revogue o dispositivo no painel.
5. A área de transferência sincroniza em tempo real nos dois sentidos. Com o celular 0.6.1-kotlin, imagens copiadas (um print, uma imagem do navegador) também vão e vêm, até 32 MB; quando a cópia tem texto e imagem, vai o texto. Para parar, desligue **Sincronização em tempo real** no painel, na bandeja ou no celular; enviar texto e arquivos continua manual.

Se a rede bloquear a descoberta (mDNS), use o QR ou **Informar endereço** no Android, com o endereço e a porta exibidos no PC. A descoberta não prova identidade: no pareamento por código, compare as cores antes de permitir. Após reinstalar ou apagar a identidade do PC, remova o pareamento antigo e pareie novamente.

## Firewall

A instalação e a atualização verificam o acesso à rede e pedem autorização de administrador do Windows quando é preciso configurar as regras. Elas são limitadas ao executável instalado: TCP 43127 para conexão e UDP 5353 para descoberta mDNS, com origem na sub-rede local, nos perfis Privado e Público. A configuração efetiva é conferida depois do reparo.

Se cancelar a autorização, o app continua instalado. Repita em **Parear celular → Permitir conexões na rede** ou em **Diagnóstico**. Regras administrativas e políticas corporativas podem exigir intervenção do administrador da rede. Rede de convidados, isolamento de clientes Wi-Fi e algumas VPNs também podem impedir o acesso entre os aparelhos.

## Privacidade e assinatura

Credenciais e chave TLS são protegidas com DPAPI do usuário. Clipboard com texto até 64 KiB UTF-8 e, desde a 0.6.1, imagens PNG ou JPEG até 32 MB, sem histórico persistente e sem conteúdo em diagnóstico. Imagens recebidas do celular vão só para a área de transferência, nunca para uma pasta. A sincronização observa flags de exclusão do Windows, mas não identifica todo conteúdo sensível. A atualização do clipboard nunca cola, executa comandos ou abre links. No MVP há um controlador conectado por PC.

Para distribuição pública, assine executável e instalador com certificado Authenticode e timestamp do provedor usando `signtool`. Configure `bundle.windows.certificateThumbprint`/`timestampUrl` localmente ou variáveis de CI seguras. Não versione PFX, senhas, credenciais ou chaves. Um instalador sem assinatura pode disparar SmartScreen. Assinatura Authenticode é independente da identidade TLS gerada por instalação.

Veja [testes e pendências](docs/TESTING.md) antes de considerar a build validada em hardware. Os pontos de extensão aceitam apenas ações tipadas e explicitamente autorizadas; não existe endpoint de shell.

## Touchpad e teclado remoto

1. Depois do pareamento o touchpad do celular já fica pronto; não há chave para ligar no PC.
2. Deslize no touchpad para mover o cursor e toque para clicar. Os botões abaixo oferecem clique direito, duplo clique, rolagem e as teclas comuns. Digite no campo de texto e toque em **Digitar**.
3. O alvo é o aplicativo em foco no Windows. Selecione a janela desejada antes de enviar texto. O controle não exibe a tela do PC.
4. Para parar, toque em **Desligar** no celular. No PC, **Interromper** (em Transferência, na Visão geral ou na bandeja) desarma na hora e **bloqueia** o mouse, o teclado e a energia pelo celular, mesmo depois de reconectar ou reiniciar, até você clicar em **Liberar**. A conexão continua para mídia, arquivos e área de transferência.

A ativação vale para a conexão: trocar de aba no Android não a desliga. Em segundo plano ou sem foco, o Android desarma o PC na hora e reativa sozinho ao voltar, na mesma conexão. Numa conexão nova o celular arma sozinho, exceto depois de **Desligar** no celular ou com o bloqueio do PC; movimentos, cliques e textos pendentes não são repetidos. Texto digitado fica somente em RAM e não entra em logs/histórico. O limite por envio é **1.024 bytes UTF-8**, sem caracteres de controle; use os botões Enter e Tab para essas teclas. Aplicativos elevados, tela de bloqueio/UAC e algumas interfaces podem recusar entrada. Teclas físicas pressionadas no PC podem interferir. Veja [INPUT.md](docs/INPUT.md) para detalhes e validações.

## Energia do PC

Com o celular 0.6.1-kotlin, **Conexão → Energia do PC** bloqueia, suspende, desliga ou reinicia o PC, agora ou com prazo (de 15 min a 2 h).

1. Ao agendar, o PC mostra uma notificação e, para prazos de 2 min ou mais, outra 60 s antes. O cartão **Energia** da Visão geral mostra a ação, o horário e quem pediu, com **Cancelar**; a bandeja tem **Cancelar ação agendada**. O celular também cancela.
2. **Interromper** bloqueia essas ações e cancela o agendamento até **Liberar**.
3. Desligar e reiniciar nunca forçam o fechamento de programas: com trabalho não salvo, o Windows pergunta. O agendamento fica só na memória; fechar o PC Remote ou reiniciar o Windows o descarta.

## Arquivos nos dois sentidos

1. **Do celular para o PC:** no Android, escolha os arquivos em **Transferir** ou use **Compartilhar** em outro app. Qualquer tipo de até 1 GB, vários de uma vez (um por vez na fila). O PC salva em **Downloads\PC Remote**, nunca sobrescreve (acrescenta `(1)`, `(2)`…) e marca cada arquivo como vindo da internet, para o SmartScreen avisar antes de rodar um programa recebido. Atalhos e arquivos que o Explorador executa só por listar a pasta (`.lnk`, `.url`, `.scf`, `.library-ms`, `desktop.ini` e similares) são recusados.
2. **Do PC para o celular:** clique em **Enviar arquivos ao celular** (Visão geral ou Transferência) ou arraste arquivos para a janela do PC Remote. Até 20 na fila, cada um de até 1 GB; eles esperam o celular conectar. O celular salva em **Download/PC Remote**.
3. O progresso aparece nos dois aparelhos e dá para cancelar dos dois lados. Uma notificação avisa quando o arquivo chega; **Abrir pasta** mostra a pasta do PC.

Cada envio usa uma conexão separada autorizada por um ticket de uso único da sessão já pareada; tamanho e SHA-256 são conferidos antes de salvar, e envios interrompidos são descartados. Os arquivos vão em blocos de 64 KiB, então a memória não cresce com o tamanho, e o PC não entra em suspensão durante a transferência. O painel guarda apenas nome, tamanho e horário dos últimos 12 arquivos, em RAM. Celulares 0.4 e 0.5 continuam enviando PNG, JPG e PDF de até 50 MB.

## Atualizações

Da 0.5.0 em diante o PC Remote se atualiza pelo próprio app. Ele lê `latest.json` da release mais recente 10 s depois de abrir e no intervalo escolhido em **Configurações → Atualizações** (1 h, 6 h, 24 h ou **Manual**; padrão 6 h).

1. Com **Baixar atualizações automaticamente** (ligado por padrão), a versão nova é baixada em segundo plano, com barra de progresso. Antes de guardar o instalador, o app confere a assinatura com a chave pública embutida e recusa arquivos adulterados ou de outra versão.
2. O cabeçalho mostra **Atualização pronta** e o cartão **Atualizações** mostra as novidades e o botão **Reiniciar para atualizar**. Nada é instalado antes desse clique. Com o download automático desligado, o botão é **Baixar e instalar**.
3. Ao reiniciar, o PC Remote fecha, o instalador roda em modo passivo (só uma barra de progresso, sem a página "já instalado" e sem UAC) e o app abre de novo. Pareamentos e configurações são mantidos; a conexão com o celular cai durante a instalação.

Se algo falhar, o cartão explica o motivo e oferece **Tentar de novo** e o link da página da versão, para baixar o instalador pelo navegador. A instalação manual da 0.5.0 sobre a 0.4.0 ou anterior é necessária uma única vez, porque essas versões não têm o atualizador. Como publicar uma versão está em [RELEASE.md](docs/RELEASE.md).

## Integração contínua

O modelo reproduzível está em [docs/ci/github-actions.yml](docs/ci/github-actions.yml). Ele executa testes e gera artefatos de avaliação. A autenticação GitHub disponível nesta entrega não possui o escopo `workflow`, portanto o modelo não está ativo. Para ativá-lo, com uma autenticação autorizada para workflows, mova o arquivo para `.github/workflows/build.yml` e publique esse commit. Os builds e testes locais estão registrados em [docs/TESTING.md](docs/TESTING.md).

A revisão de 15/10/2026 está documentada na [auditoria de segurança](docs/SECURITY-AUDIT.md) e no [relatório de bugs e regressões](docs/BUG-AUDIT-2026-10-15.md), com evidências e limites da validação.
