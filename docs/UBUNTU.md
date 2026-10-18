# Instalar no Ubuntu

O empacotamento tem como alvo **Ubuntu 22.04, 24.04 e 26.04 LTS, amd64**. O caminho recomendado é o `.deb`: o APT instala as bibliotecas necessárias e o aplicativo aparece no menu. Não é necessário instalar Node, Rust ou ferramentas de desenvolvimento no computador que vai usar o app.

## Instalação e atualização

Baixe o [.deb amd64](https://github.com/danielharrisonlf/pc-remote-desktop/releases/download/v0.6.4-ubuntu.1/pc-remote-desktop-0.6.4-amd64.deb) ou o [instalador compactado](https://github.com/danielharrisonlf/pc-remote-desktop/releases/download/v0.6.4-ubuntu.1/pc-remote-ubuntu-0.6.4-amd64.tar.gz) na [pré-release Ubuntu v0.6.4-ubuntu.1](https://github.com/danielharrisonlf/pc-remote-desktop/releases/tag/v0.6.4-ubuntu.1). A versão do aplicativo e do pacote Debian é **0.6.4**; o sufixo da tag identifica a primeira distribuição Ubuntu desta revisão. A instalação e a abertura passaram nas três versões LTS, enquanto o uso em um desktop GNOME Wayland real ainda exige validação.

Com o `.deb` desta build em Downloads, execute (ajuste o nome se necessário):

```bash
sudo apt install --no-install-recommends ~/Downloads/pc-remote-desktop-0.6.4-amd64.deb
```

Outra opção é extrair `pc-remote-ubuntu-0.6.4-amd64.tar.gz`, abrir o terminal na pasta extraída e executar:

```bash
bash install-ubuntu.sh
```

O script identifica a versão do Ubuntu, confere a identidade e a arquitetura do pacote e instala as dependências obrigatórias com o APT, sem trazer recomendações opcionais do ambiente gráfico. Também aceita `bash install-ubuntu.sh /caminho/pacote.deb`. O terminal pede a senha administrativa para instalar; depois, abra **PC Remote** no menu, como usuário normal.

Para conferir os arquivos extraídos, execute `sha256sum -c SHA256SUMS` antes de instalar. Esse checksum detecta corrupção do download; não substitui uma assinatura do publicador.

Para atualizar, encerre o PC Remote pela bandeja e instale o novo `.deb` da mesma forma. Pareamentos e configurações são mantidos. O atualizador automático documentado para Windows não distribui atualizações `.deb`.

## Ubuntu 26.04 e controle pelo celular

O Ubuntu Desktop 26.04 usa GNOME com Wayland. O PC Remote usa o portal de controle remoto do sistema para enviar mouse e teclado aos aplicativos nativos Wayland, sem exigir uma sessão Xorg, serviço privilegiado ou acesso a `/dev/uinput`.

1. Abra **Configurações → Controle no Ubuntu → Autorizar controle** no PC Remote.
2. Na janela do Ubuntu, permita mouse e teclado. Não é necessário compartilhar a tela.
3. Pareie o celular e use o touchpad e o teclado normalmente.

A autorização vale para a sessão do aplicativo e pode ser solicitada novamente depois de reabri-lo. Cancelar o pedido ou encerrar o compartilhamento no sistema bloqueia a entrada; clique novamente em **Autorizar controle** para liberar. Eventos enviados pelo celular não abrem janelas de permissão. O botão **Interromper** continua bloqueando a entrada pelo celular.

O pacote inclui a dependência do portal e um backend GNOME ou KDE, além do XWayland para interoperabilidade do clipboard. No Ubuntu 22.04/24.04 com sessão X11, a entrada continua usando XTEST. Áudio usa `pactl` com PulseAudio ou PipeWire; arquivos, mídia e energia continuam sujeitos às permissões da sessão e às capacidades de cada aplicativo.

## Gerar os instaladores (mantenedor)

Compile no **Ubuntu 22.04**, a base mais antiga suportada, para produzir o mesmo `.deb` para as três versões LTS. Uma build feita em 26.04 pode exigir uma glibc nova demais para 22.04.

Instale Node.js LTS >=22.12, Rust estável e as dependências de compilação:

```bash
sudo apt update
sudo apt install build-essential pkg-config curl wget file libssl-dev \
  libwebkit2gtk-4.1-dev libxdo-dev libayatana-appindicator3-dev librsvg2-dev
npm run build:ubuntu
```

O comando instala as dependências npm bloqueadas, compila o aplicativo com frontend embutido, gera o `.deb` e prepara em `release-assets/0.6.4/`:

- `ubuntu-amd64/pc-remote-desktop-0.6.4-amd64.deb`;
- `ubuntu-amd64/install-ubuntu.sh`, instruções e `SHA256SUMS`;
- `pc-remote-ubuntu-0.6.4-amd64.tar.gz`, com os mesmos arquivos.

`src-tauri/tauri.linux.conf.json` desativa os artefatos do atualizador exclusivamente no Linux: gerar o `.deb` não exige a chave de assinatura Windows. A distribuição principal é `.deb`; AppImage pode ser gerado separadamente com `npm run tauri -- build --bundles appimage`, mas exige validação própria.

No Windows com Docker Desktop, a validação também produz o `.deb` e o arquivo `.tar.gz`:

```powershell
powershell -File scripts/linux-validation.ps1
# Compilação e testes numa base mais nova, quando necessário:
powershell -File scripts/linux-validation.ps1 -UbuntuVersion 26.04
```

Antes de distribuir, instale **o mesmo pacote compilado em 22.04** em cada versão LTS. Os testes isolados e seus limites estão em [linux-validation.md](linux-validation.md). Uma sessão Xvfb ou portal simulado não valida o consentimento e a entrega de entrada em um desktop GNOME Wayland real.

Referências: [notas do Ubuntu 26.04](https://documentation.ubuntu.com/release-notes/26.04/), [API oficial do portal RemoteDesktop](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html), [pacotes Debian no Tauri](https://v2.tauri.app/distribute/debian/).
