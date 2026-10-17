# Revisão de dependências — 15/10/2026

Aplicativos: Windows 0.2.2, Android React Native 0.2.1 e Kotlin 0.2.4-kotlin. Lockfiles e Gradle registram as versões exatas. Esta revisão combina auditoria de dependências com inspeção do código; não equivale a uma certificação de ausência de vulnerabilidades.

## Correção aplicada

Rustls foi atualizado de 0.23.45 para **0.23.46**, com alteração restrita no Cargo.lock. As quatro notificações publicadas em 14/10/2026 têm correção nessa versão: [RUSTSEC-2026-0346](https://rustsec.org/advisories/RUSTSEC-2026-0346.html), [0347](https://rustsec.org/advisories/RUSTSEC-2026-0347.html), [0348](https://rustsec.org/advisories/RUSTSEC-2026-0348.html) e [0349](https://rustsec.org/advisories/RUSTSEC-2026-0349.html).

Os casos envolvem depuração de sessões TLS de cliente, ECH, descompressão Brotli opcional e cache de cliente pós-quântico. O produto usa servidor TLS sem autenticação de certificado de cliente; o ClientConfig é de teste e a funcionalidade Brotli não está habilitada. Mesmo assim, o componente foi corrigido. Consulta oficial OSV dirigida a rustls 0.23.46 retornou zero avisos na data da revisão.

## Resultados e pendências

| Consulta | Resultado | Tratamento |
|---|---|---|
| npm audit desktop | Zero vulnerabilidades informadas | Lockfile preservado |
| npm audit React Native | 53 nós afetados: 48 high e 5 moderate, propagados por dois avisos | Não são 53 vulnerabilidades distintas; ver abaixo |
| OSV do Cargo.lock | Consulta de 518 crates públicas por nome/versão | Rustls corrigido; avisos residuais abaixo |
| Kotlin nativo | Sem npm; versões diretas fixas no Gradle | Não foi executado um scanner completo de todas as dependências Maven |

No React Native, **braces 3.0.3** ([GHSA-vfj7-8cjw-p6xm](https://github.com/advisories/GHSA-vfj7-8cjw-p6xm)) e **sprintf-js 1.0.3** ([GHSA-hp3w-g68c-fv3c](https://github.com/advisories/GHSA-hp3w-g68c-fv3c)) continuam afetados. Os avisos e o registro npm não oferecem versão publicada corrigida na data desta revisão. Atualizar sprintf-js para 1.1.3 não resolve o segundo aviso. Os caminhos identificados pertencem às ferramentas Jest/Metro/cobertura; não foi demonstrado um caminho de exploração por mensagens do aplicativo. Não se afirma que o risco seja nulo. Evite entradas de build/teste de origem não confiável e acompanhe as correções upstream. Não foi aplicado `npm audit fix`, cuja proposta inclui downgrade incompatível do React Native.

No grafo Rust permanecem avisos de manutenção: [proc-macro-error 1.0.4](https://rustsec.org/advisories/RUSTSEC-2024-0370.html) e [rustls-pemfile 2.2.0](https://rustsec.org/advisories/RUSTSEC-2025-0134.html). O segundo continua usado para PEM; o aviso de abandono não descreve por si só uma exploração. O aviso [glib 0.18.5](https://rustsec.org/advisories/RUSTSEC-2024-0429.html) está no lockfile multiplataforma, mas não no grafo de build Windows selecionado; proc-macro-error também não aparece nesse grafo. Nenhuma atualização ampla de Tauri foi feita nesta correção.

## Proteções revisadas no código

- Persistência de credenciais antes de publicar um novo pareamento; restauração do pareamento anterior quando a gravação falha.
- Revogação interrompe a sessão e bloqueia reconexão no processo mesmo quando o armazenamento falha. O erro exige nova tentativa: reiniciar antes de salvar a revogação pode restaurar o acesso antigo. Um novo pareamento exige aprovação e gravação bem-sucedida.
- Clipboard Android valida a descrição do mesmo ClipData que fornece o texto. Conteúdo marcado sensível não é lido para sincronização. Isso não detecta todas as senhas.
- Clipboard Windows confere a sequência sob OpenClipboard antes de uma escrita automática; uma alteração local pendente é preservada.
- Nenhum conteúdo de clipboard ou credencial foi incluído nos relatórios de testes. Consultas OSV enviaram somente nomes e versões de crates públicas.

## Recebimento de imagens e PDFs (files.v1) — 16/10/2026

Modelo de ameaças considerado: outro aparelho na LAN, celular pareado com cliente defeituoso ou malicioso e nome/conteúdo hostil vindo do celular.

- **Desligado por padrão.** `allowFiles` não existe em preferências antigas e começa falso; desligá-lo aborta a transferência em curso. Clientes sem files.v1 não veem capacidade nem campo novo.
- **Ticket.** O socket `/v1/files` não aceita credencial própria: só um ticket de 32 bytes aleatórios emitido pela sessão de controle autenticada, válido por 30 s, de uso único, comparado em tempo constante e ligado a dispositivo, epoch, transferId, nome, tamanho e MIME. Qualquer `file.begin` que cite o transferId consome o ticket, mesmo com segredo errado, o que impede tentativas repetidas. Fim, troca ou revogação da sessão invalidam ticket e transferência; a autorização é conferida a cada frame.
- **Limites.** No máximo um ticket ou envio por PC, 50 MiB por arquivo, frames de 64 KiB, texto de 4 KiB, 10 s para `file.begin`, 20 s sem frames, 15 min por envio, fila de 16 blocos até o disco e espaço livre mínimo de tamanho + 64 MiB. Bytes além do declarado são recusados imediatamente.
- **Tipo e integridade.** Só PNG, JPEG e PDF, com MIME, extensão e assinatura coerentes, e SHA-256 conferido antes de qualquer nome visível. A assinatura não prova que o arquivo é inofensivo: PDFs e imagens ainda são abertos por leitores do Windows.
- **Mark-of-the-Web.** Todo arquivo salvo recebe `Zone.Identifier` com ZoneId=3, para que SmartScreen, Office e leitores tratem o arquivo como vindo da internet. Se o volume não suportar fluxos alternativos (por exemplo FAT32), o envio falha com `storage` em vez de salvar sem a marca.
- **Sem sobrescrita nem fuga de pasta.** O nome é reduzido ao último componente e sanitizado (caracteres proibidos, controles, marcas bidirecionais, nomes de dispositivo reservados, pontos/espaços finais, 100 caracteres). A parte é criada com `create_new`; o nome final usa `MoveFileExW` sem `MOVEFILE_REPLACE_EXISTING` (nunca `std::fs::rename`, que substitui no Windows). A pasta `Downloads\PC Remote` é recusada se for link simbólico, junção ou outro reparse point.
- **Privacidade.** O painel guarda só nome, tamanho e horário dos últimos 12 arquivos, em RAM. Diagnósticos de falha não incluem conteúdo, nome do arquivo nem caminho completo.

Limites conhecidos: um processo local com os mesmos privilégios do usuário pode trocar a pasta entre a verificação e a gravação; isso não amplia o acesso desse processo. A abertura do arquivo continua sendo uma decisão do usuário; nada é executado ou aberto automaticamente.

## Atualização pelo app (0.5.0) — 16/10/2026

Modelo de ameaças considerado: rede hostil entre o PC e o GitHub, manifesto `latest.json` adulterado ou trocado, instalador adulterado, release antiga republicada, conteúdo hostil nas notas e o próprio painel tentando abusar do atualizador.

- **Assinatura obrigatória.** `tauri-plugin-updater` 2.13.3 (minisign-verify 0.3.0) verifica a assinatura Ed25519 do instalador com a chave pública fixa em `tauri.conf.json` (ID `DF723D16C1AD8F8A`) antes de devolver os bytes ao Rust; arquivos que falham são descartados e o painel diz "A assinatura da atualização não confere". O manifesto não é assinado, então a assinatura é a garantia principal.
- **Versão assinada.** `requireSignedVersion` está ligado: o `tauri build` grava `version:<versão>` no comentário confiável (coberto pela assinatura) e o plugin recusa um manifesto que anuncie outra versão para aquele arquivo. Isso impede reaproveitar uma release antiga, validamente assinada, com um número novo. Assinaturas sem versão também são recusadas.
- **Sem downgrade.** O plugin só oferece versões maiores que a em execução (semver), e `update_install` confere de novo antes de executar.
- **Transporte.** HTTPS com rustls e as raízes do Windows (`rustls-platform-verifier`); `http://` só é aceito em builds de debug, junto com `PCREMOTE_UPDATE_ENDPOINT`, que builds de release ignoram. O provedor criptográfico do processo continua aws-lc-rs.
- **Menor privilégio no painel.** A WebView não tem permissões `updater:*`; só chama `update_check`, `update_download` e `update_install`, sem URL, caminho ou argumento livre. O download acontece no Rust; a CSP perdeu `https://api.github.com`, que não é mais consultado. As notas (até 20.000 caracteres) passam pelo renderizador Markdown próprio, sem HTML; só links de releases do projeto podem ser abertos, e `open_url` mantém a allowlist.
- **Instalação.** O instalador por usuário roda em modo passivo (`/P /UPDATE /R`) sem UAC, depois que o usuário clica em **Reiniciar para atualizar**. Ele é gravado em `%TEMP%` pelo plugin; um processo local com os mesmos privilégios poderia trocá-lo entre a gravação e a execução, o que não amplia o acesso desse processo. O arquivo não recebe Mark-of-the-Web, então o SmartScreen não intervém.
- **Custódia da chave.** A chave privada (sem senha) fica em `%USERPROFILE%\.tauri\pc-remote-desktop-updater.key`, fora do repositório; o `.gitignore` cobre chaves, senhas e `release-assets/`. Quem obtiver a chave **e** conseguir publicar uma release (ou responder pelo endpoint) consegue distribuir um instalador aceito: proteja a conta do GitHub com 2FA e mantenha backup offline da chave. Perder a chave obriga uma reinstalação manual em cada PC. Procedimento em [RELEASE.md](RELEASE.md).
- **Dependências novas.** Só adições ao `Cargo.lock` (plugin, minisign-verify, hyper-rustls, rustls-platform-verifier e dependências de outras plataformas); nenhuma versão existente mudou.

APKs continuam assinados com chave debug de avaliação; o instalador Windows não possui Authenticode. Assinatura de produção, testes físicos/OEM, leitura óptica de QR e revisão externa de segurança permanecem pendentes. Consulte [testes](TESTING.md) e [revisão de bugs](BUG-AUDIT-2026-10-15.md).

## Menos chaves, arquivos de até 1 GB nos dois sentidos e pesquisa no YouTube (0.6.0) — 16/10/2026

Mudança de postura pedida pelo dono do produto: o que funciona depois do pareamento passa a funcionar sem chaves no PC. O pareamento continua exigindo código de 8 dígitos válido por 120 s e aprovação no PC; token individual, pinning do certificado, revogação e validação estrita não mudaram.

- **Sem `allowFiles` e `allowInput`.** Um celular pareado envia arquivos e usa mouse/teclado sem liberar antes. A defesa contra controle indesejado passa a ser **Interromper** (painel ou bandeja): desativa a sessão na hora, publica `allowed:false` e grava `inputBlocked` em `settings.json`, de modo que nem um `input.enable` na fila nem uma reconexão desfazem o bloqueio; só **Liberar** no PC o remove. Se a gravação falhar, o bloqueio vale na execução e o painel avisa. Revogar o celular continua sendo o caminho para retirar todo acesso.
- **Qualquer tipo de arquivo (files.v2).** Só com a extensão negociada; clientes files.v1 continuam com PNG/JPG/PDF e 50 MB. Mantidos: ticket de 30 s de uso único ligado ao epoch, SHA-256, nome limpo, `create_new`, movimento sem substituir, pasta comum (sem reparse point) e `Zone.Identifier` ZoneId=3 em todo arquivo — o SmartScreen avisa antes de rodar um programa recebido e o Office abre em Modo de Exibição Protegido. Nada é aberto ou executado automaticamente. Recusados mesmo com files.v2, porque o Explorador age sobre eles só por listar a pasta (ícones remotos, atalhos): `lnk`, `url`, `scf`, `library-ms`, `searchconnector-ms`, `settingcontent-ms`, `appref-ms`, `website` e `desktop.ini`. A lista não pretende cobrir todo formato perigoso; ela cobre o que não depende de o usuário abrir o arquivo.
- **Arquivos grandes.** 1 GiB por arquivo; espaço livre exigido de tamanho + 64 MiB; tempo total de no máximo 15 min ou 1 s por 256 KiB; 20 s sem progresso abortam. Fila de 16 blocos até o disco e janela de 1 MiB: a memória usada não cresce com o tamanho do arquivo. Um pedido de energia (`PowerSetRequest`) impede a suspensão só enquanto há transferência.
- **PC → celular.** O celular só recebe o que o usuário escolheu no PC (seletor nativo ou arrastar para a janela); o caminho fica no Rust, a WebView recebe só nome e tamanho. O `file.push` vai apenas para a sessão autenticada com files.v2, com ticket próprio de 30 s, uso único, comparação em tempo constante e vínculo ao dispositivo e ao epoch; um `file.fetch` errado consome a oferta. O PC lê exatamente o tamanho anunciado e aborta se o arquivo mudar; o celular confere o SHA-256 antes de expor o arquivo. Cancelamento em qualquer lado apaga a parte no celular.
- **Pesquisa (search.v1).** O celular manda só o texto (até 200 caracteres, sem controles). O PC monta a URL com origem fixa `https://www.youtube.com/results?search_query=` e codifica tudo como formulário, então `&`, `#`, `/`, `?` e esquemas não escapam do parâmetro; o limite é de 5 pesquisas em 10 s. O resultado é uma aba do navegador padrão; nenhum endereço vindo do celular é aberto.
- **Clipboard em tempo real ligado por padrão.** Continua sem histórico e com bloqueio de conteúdo marcado como sensível (`ExcludeClipboardContentFromMonitorProcessing`, `CanUploadToCloudClipboard=0` no Windows; `EXTRA_IS_SENSITIVE` no Android). Isso não detecta toda senha: quem não quer sincronizar desliga **Sincronização em tempo real** (painel ou bandeja). Envios manuais não dependem da opção.
- **Superfície removida.** O painel não mostra mais endereço e identidade na Visão geral (continuam em Diagnóstico, onde servem à comparação de blocos).

## Energia do PC e imagens na área de transferência (0.6.1) — 16/10/2026

As duas extensões são negociadas (`power.v1` e `clipboard.v2`): celulares que não as pedem recebem exatamente o formato 0.6.0. Pareamento, token individual, pinning do certificado e revogação não mudaram.

- **Energia (power.v1).** Só o celular pareado e autenticado da conexão que negociou a extensão pede bloquear, suspender, desligar ou reiniciar, agora ou em até 24 h. **Interromper** bloqueia essas ações e cancela o agendamento; com ele ativo, `power.run` recebe `unavailable` e uma ação agendada é descartada no prazo. Há um agendamento por PC, só na memória (fechar o PC Remote ou reiniciar o Windows o descarta), cancelável pelo painel, pela bandeja, por `power.cancel` de qualquer celular pareado, por Interromper e pela revogação do celular que agendou. O PC notifica ao agendar ("Desligar às 23:40, pedido por <celular>") e 60 s antes dos prazos de 2 min ou mais. Limite de 3 pedidos em 10 s.
- **Sem forçar.** Desligar e reiniciar usam `InitiateShutdownW` sem `SHUTDOWN_FORCE_*`: programas com trabalho não salvo podem segurar, e o Windows mostra a própria tela. O privilégio `SeShutdownPrivilege` é habilitado no token só no momento de suspender, desligar ou reiniciar. Uma falha vira diagnóstico e notificação, sem nova tentativa.
- **Imagens na área de transferência (clipboard.v2).** Mesmo consentimento do texto: envio automático só com **Sincronização em tempo real** ligada no PC e a direção pedida pelo celular; envios manuais não dependem dela. Conteúdo marcado como sensível não sai, e o texto tem prioridade quando a cópia tem os dois. Só PNG ou JPEG de até 32 MiB e 50 megapixels; a imagem do celular é decodificada pelo WIC antes de entrar na área de transferência (`CF_DIBV5` e `PNG`), e uma escrita automática exige que a área de transferência não tenha mudado desde a oferta.
- **Nada vira arquivo.** Imagens da área de transferência passam por `%TEMP%\PC Remote` (recusada se não for pasta comum), são apagadas depois de cada envio, e sobras de execuções anteriores são limpas ao iniciar. Não vão para Downloads nem recebem `Zone.Identifier`, porque não viram arquivo. Esses envios ficam fora da fila e do histórico de arquivos do painel; qualquer cópia nova no PC descarta um envio de imagem ainda não concluído.
- **Eco corrigido.** O PC guardava o número de sequência da área de transferência lido antes de `CloseClipboard`, que acrescenta os formatos sintetizados e avança o número. Uma escrita feita a pedido do celular parecia então uma cópia nova no Windows e voltava ao celular (com texto também, desde a 0.6.0). Agora a escrita devolve o número lido depois de fechar, e os testes nativos de área de transferência conferem isso.
