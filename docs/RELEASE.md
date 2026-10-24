# Publicar uma versão

## Distribuição Ubuntu

O guia [UBUNTU.md](UBUNTU.md) descreve a geração dos instaladores e a validação do mesmo `.deb` nas três versões LTS. A primeira distribuição usa a tag `v0.6.4-ubuntu.1`, mantendo a versão 0.6.4 embutida nos pacotes já validados. Publique `.deb`, `.tar.gz`, `install-ubuntu.sh` e `UBUNTU-SHA256SUMS-0.6.4.txt` com as [notas Ubuntu](releases/0.6.4-ubuntu.1.md).

Releases específicas de Ubuntu devem ser publicadas com `--latest=false`: o endpoint `/releases/latest/download/latest.json` atende o atualizador Windows e precisa continuar apontando para uma release com seu manifesto assinado. Esta distribuição Ubuntu também usa `--prerelease`, pois a validação em uma sessão GNOME Wayland real permanece pendente. A tag deve apontar para o commit publicado com estas alterações; nunca mova a tag Windows existente para anexar código novo.

## Distribuição Windows

A partir da 0.5.0 o PC Remote se atualiza sozinho: lê `latest.json` da release mais recente, baixa o instalador em segundo plano, confere a assinatura e, quando o usuário clica em **Reiniciar para atualizar**, roda o instalador em modo passivo e abre de novo. Para isso cada release precisa ser assinada com a chave do atualizador e publicar quatro arquivos. Este documento descreve o processo completo.

## Chave de assinatura do atualizador

| Item | Valor |
|---|---|
| Chave privada | `%USERPROFILE%\.tauri\pc-remote-desktop-updater.key` |
| Chave pública | `%USERPROFILE%\.tauri\pc-remote-desktop-updater.key.pub`, copiada em `src-tauri/tauri.conf.json` → `plugins.updater.pubkey` |
| ID da chave (minisign) | `DF723D16C1AD8F8A` |
| Senha | Nenhuma (a chave foi gerada com senha vazia) |

> **Faça backup da chave privada agora**, fora deste PC (pendrive criptografado ou gerenciador de senhas). Se ela for perdida, as instalações existentes não aceitam mais nenhuma atualização: será preciso gerar outra chave, publicar uma versão com a nova `pubkey` e cada usuário terá de baixar e instalar essa versão **manualmente uma vez**.

- A chave privada e qualquer senha **nunca** entram no repositório, em issues, em logs de CI ou em mensagens. O `.gitignore` cobre `*.key`, `*.key.pub`, `*.password.txt`, `.tauri/` e `release-assets/`, mas não confie só nele.
- Quem tiver a chave privada consegue produzir um instalador que o app aceita. Se suspeitar de vazamento, gere outra chave (`npx tauri signer generate -w <novo caminho>`), troque a `pubkey`, publique uma nova versão e avise que ela precisa de instalação manual.
- A assinatura do atualizador não é Authenticode. O instalador continua sem certificado de código; o Windows não mostra aviso na atualização pelo app (o arquivo não tem Mark-of-the-Web), mas pode mostrar no download pelo navegador.

## Variáveis de ambiente

| Variável | Uso |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | Caminho do arquivo da chave **ou** o conteúdo dele. Obrigatória: com `bundle.createUpdaterArtifacts` ligado, a build falha sem ela. |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Senha da chave. A chave atual não tem senha: deixe a variável indefinida. |

Para uma build local sem a chave (só para testar o instalador, sem publicar), use `npm run tauri build -- --target x86_64-pc-windows-msvc --config <arquivo.json>` com um arquivo contendo `{ "bundle": { "createUpdaterArtifacts": false } }`. Esse instalador não serve para a release.

## Passo a passo

1. **Versão.** Atualize a mesma versão em `package.json`, `package-lock.json` (campo `version` e `packages[""].version`), `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json` e a entrada `pc-remote-desktop` de `src-tauri/Cargo.lock` (um `cargo check` atualiza). O script de release recusa versões divergentes. A versão precisa ser maior que a publicada: o app só oferece versões mais novas.
2. **Testes.**

   ```powershell
   npm ci
   npx tsc --noEmit
   npm test
   cd src-tauri
   cargo fmt --check
   cargo test --target x86_64-pc-windows-msvc
   cd ..
   ```

3. **Build assinada.**

   ```powershell
   $env:TAURI_SIGNING_PRIVATE_KEY = "$env:USERPROFILE\.tauri\pc-remote-desktop-updater.key"
   npm run tauri build -- --target x86_64-pc-windows-msvc
   ```

   Em `src-tauri\target\x86_64-pc-windows-msvc\release\bundle\nsis\` ficam `PC Remote_<versão>_x64-setup.exe` e `PC Remote_<versão>_x64-setup.exe.sig`. O `tauri build` grava a versão no comentário confiável da assinatura; o app exige isso (`requireSignedVersion`). Não assine à mão sem `--app-version`.
4. **Notas.** Escreva as novidades em Markdown num arquivo fora do repositório ou descartável (por exemplo `notas-0.5.1.md`). O texto vai para `latest.json` e aparece no painel e no diálogo de atualização. Use listas, negrito, `código` e links das releases do projeto; outros links aparecem como texto.
5. **Manifesto e checksums.**

   ```powershell
   npm run release:updater -- --notes-file notas-0.5.1.md
   ```

   O script (`scripts/release-updater.mjs`, sem dependências) confere a assinatura com a `pubkey` de `tauri.conf.json` e a versão assinada, copia o instalador com o nome publicado `pc-remote-desktop-<versão>-x64-setup.exe` (renomear não altera os bytes assinados, então o `.sig` continua válido) e grava em `release-assets\<versão>\`:

   | Arquivo | Conteúdo |
   |---|---|
   | `pc-remote-desktop-<versão>-x64-setup.exe` | Instalador NSIS por usuário |
   | `pc-remote-desktop-<versão>-x64-setup.exe.sig` | Assinatura minisign do instalador |
   | `latest.json` | `version`, `notes`, `pub_date` (RFC 3339) e `platforms["windows-x86_64"]` / `["windows-x86_64-nsis"]` com `signature` e `url` `https://github.com/danielharrisonlf/pc-remote-desktop/releases/download/v<versão>/pc-remote-desktop-<versão>-x64-setup.exe` |
   | `WINDOWS-SHA256SUMS-<versão>.txt` | SHA-256 dos três arquivos acima |

   Opções: `--notes "<texto>"`, `--bundle-dir <pasta>`, `--out <pasta>`, `--pub-date <RFC 3339>`.
6. **Conferência opcional com o verificador do próprio plugin**, sem instalar nada:

   ```powershell
   $env:PCREMOTE_UPDATER_E2E_DIR = (Resolve-Path release-assets\0.5.1).Path
   cd src-tauri
   cargo test --target x86_64-pc-windows-msvc --test updater -- --ignored --nocapture
   ```

   O teste serve esses arquivos em `127.0.0.1`, faz a verificação e o download como o app (simulando uma instalação mais antiga, com as configurações reais de `tauri.conf.json`) e confirma que uma cópia adulterada é recusada.
7. **Release no GitHub.** Crie a tag `v<versão>` e a release, envie os **quatro** arquivos com esses nomes exatos (o manifesto precisa se chamar `latest.json`) e marque a release como **Latest**, nem rascunho nem pré-release: o app consulta `https://github.com/danielharrisonlf/pc-remote-desktop/releases/latest/download/latest.json`.
8. **Verificação pós-publicação.** `curl -L https://github.com/danielharrisonlf/pc-remote-desktop/releases/latest/download/latest.json` deve mostrar a versão nova. Num PC com a versão anterior, **Verificar agora** em Configurações → Atualizações encontra a versão.

Uma release sem `latest.json` deixa o app sem atualização (ele informa "Nenhuma atualização publicada foi encontrada"). Para retirar uma versão problemática, publique outra com número maior; o app não instala versões mais antigas.

## O que acontece no PC do usuário

Desde a 0.6.8, o hook `src-tauri/windows/network-hooks.nsh` executa o modo interno `--configure-network` depois da cópia dos arquivos, tanto na instalação quanto na atualização `/UPDATE`. Esse modo é tratado antes do Tauri e do serviço de rede; ele compartilha a rotina do botão de reparo e resolve internamente o caminho do executável. O Windows solicita UAC quando a configuração exige elevação.

Os códigos de saída são 0 para permissões efetivas verificadas, 2 para cancelamento do UAC e 3 para restrição ou falha de verificação. Cancelamento ou falha mantêm o aplicativo instalado e orientam a repetir o reparo pelo painel. O hook aguarda a conclusão antes da abertura normal do app. A integração usa o hook pós-instalação documentado pelo [Tauri](https://v2.tauri.app/distribute/windows-installer/#extending-the-installer).

`powershell -File scripts/test-network-installer.ps1` compila o hook real com um executável de teste inofensivo e verifica instalação, atualização, cancelamento, restrição e helper ausente. Esse teste não executa UAC nem altera o firewall; a rotina de permissões tem testes próprios de planejamento e execução no PowerShell com APIs de rede substituídas em memória.

1. O app verifica 10 s depois de abrir e depois no intervalo escolhido (1 h, 6 h ou 24 h; padrão 6 h; **Manual** só verifica pelo botão). Uma verificação que falhou é repetida em 15 min.
2. Com **Baixar atualizações automaticamente** (padrão), o Rust baixa o instalador e confere a assinatura e a versão assinada antes de guardá-lo em memória. O painel mostra o progresso e depois **Reiniciar para atualizar**; o cabeçalho mostra **Atualização pronta**. Sem essa opção, o botão é **Baixar e instalar**.
3. Ao reiniciar, o Rust grava o instalador em `%TEMP%`, inicia `setup.exe /P /UPDATE /R` e encerra o processo (o ícone da bandeja é removido antes). O NSIS em `/UPDATE` pula a página "já instalado", fecha pelo Restart Manager uma instância que ainda esteja aberta e instala por usuário. A etapa de permissões solicita UAC se precisar configurar o firewall; depois o app abre de novo. Configurações, pareamentos e identidade em `%APPDATA%\PC Remote` são mantidos.

A versão 0.4.0 e anteriores não têm atualizador: a 0.5.0 é instalada manualmente uma vez, por cima da instalação existente (na página do NSIS, mantenha a opção padrão e não marque a exclusão dos dados do aplicativo).

## Testar o atualizador localmente

- Builds de debug aceitam `PCREMOTE_UPDATE_ENDPOINT=<url do latest.json>` para apontar a um servidor local (`http://` permitido só em debug). Builds de release ignoram a variável.
- O plugin Single Instance faz um segundo executável sair na hora se o PC Remote instalado estiver aberto. Para testar a lógica sem fechar a instalação, use os testes de `src-tauri/tests/updater.rs` (rodam por padrão com fixtures próprias; o teste `--ignored` usa os artefatos reais).
- A instalação NSIS `/UPDATE` em si só pode ser testada substituindo uma instalação real; faça isso numa máquina ou conta de teste.
