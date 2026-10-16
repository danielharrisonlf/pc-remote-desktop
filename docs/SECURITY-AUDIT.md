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

APKs continuam assinados com chave debug de avaliação; o instalador Windows não possui Authenticode. Assinatura de produção, testes físicos/OEM, leitura óptica de QR e revisão externa de segurança permanecem pendentes. Consulte [testes](TESTING.md) e [revisão de bugs](BUG-AUDIT-2026-10-15.md).
