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

APKs continuam assinados com chave debug de avaliação; o instalador Windows não possui Authenticode. Assinatura de produção, testes físicos/OEM, leitura óptica de QR e revisão externa de segurança permanecem pendentes. Consulte [testes](TESTING.md) e [revisão de bugs](BUG-AUDIT-2026-10-15.md).
