# Compatibilidade de mídia

Backend: Windows GlobalSystemMediaTransportControlsSessionManager (GSMTC), disponível a partir de Windows 10 1809. A UI só habilita a ação se a sessão atual declarar suporte; o Rust consulta novamente antes de executar. Um retorno `false` da API é erro de execução, não sucesso.

| Integração | Play/pause | Faixa anterior/próxima | Posição −10s/+10s | Evidência/limite |
|---|---|---|---|---|
| Sessões GSMTC | Condicional a IsPlayEnabled/IsPauseEnabled | Condicional a IsPreviousEnabled/IsNextEnabled | Condicional a IsPlaybackPositionEnabled e timeline válida | Implementação nativa; depende do que cada app publica |
| Navegadores com Media Session | Apenas quando expõem sessão Windows | Apenas quando o site publica handlers | Apenas se o site/browser expõe seek na sessão | Não há integração DOM ou controle universal de vídeos |
| Apps de música com sessão Windows | Conforme flags reais | Conforme flags reais | Conforme flags/timeline | Nenhum nome comercial implica compatibilidade garantida |
| Players sem GSMTC | Indisponível | Indisponível | Indisponível | Sem adaptador específico no MVP |
| Teclado | Desativado | Desativado | Desativado | Não envia atalhos globais que poderiam atingir outro aplicativo |

Título e aplicativo são mostrados quando a sessão fornece esses dados. Cada sessão tem ID distinto; a seleção Android fixa o alvo da ação. Sessão encerrada resulta em indisponível. Navegação de faixa não é usada como substituto de seek. O seek soma/subtrai 10s da timeline atual e limita ao intervalo que o player informa; unidades Windows são ticks de 100ns.

Não há ainda matriz certificada em aparelhos/players reais. Registre em [TESTING.md](TESTING.md) o aplicativo, versão, Windows build, tipo de mídia, flags e resultado. Os controles não prometem funcionar em todos os vídeos. [API oficial de seek](https://learn.microsoft.com/en-us/uwp/api/windows.media.control.globalsystemmediatransportcontrolssession.trychangeplaybackpositionasync), [flags oficiais](https://learn.microsoft.com/en-us/uwp/api/windows.media.control.globalsystemmediatransportcontrolssessionplaybackcontrols).

## Aplicativos 0.6.0 — 16/10/2026

- **Sessão certa por padrão.** O PC lista primeiro a sessão que o Windows trata como atual (`GetCurrentSession`), depois as que estão tocando. O Android segue a primeira sessão enquanto o usuário não tocar em outra. Antes, a primeira da lista do Windows podia ser um player parado, e os comandos iam para ele.
- **Identidade estável.** O worker mantém um único `GlobalSystemMediaTransportControlsSessionManager`; as sessões conservam o UUID entre leituras (teste ignorado `windows_gsmtc_session_ids_are_stable_across_reads`, conferido com Teams, Chrome e Reprodutor Multimídia).
- **Estado sem atraso aparente.** Depois de um comando aceito o PC relê as sessões em 150, 500 e 1.200 ms; o Android mostra o estado esperado de reproduzir/pausar até o snapshot confirmar (até 2,5 s). Só mudar a posição não gera snapshot: nenhuma tela a mostra, e isso evitava um snapshot a cada 650 ms durante a reprodução.
- **Nomes legíveis.** `app` vira "Spotify", "Google Chrome", "Microsoft Edge", "Firefox", "Reprodutor Multimídia", "Microsoft Teams" etc. quando o AppUserModelID é conhecido.

| Player (Windows 11 Pro, build 26300) | Sessão | Play/pause | Anterior/próxima | ±10 s | Resultado |
|---|---|---|---|---|---|
| Reprodutor Multimídia 11.2607 (playlist de 3 WAV) | sim, primeira da lista | ok pelo celular | ok (silencio-1 ↔ silencio-2) | ok | O app publica a linha do tempo a cada ~6 s: o efeito do seek aparece na atualização seguinte |
| YouTube no Google Chrome | sim, primeira da lista, "Google Chrome" | anunciados pela sessão (`pause`, `previous`, `next`, `seek`) | anunciados (playlist/mix) | anunciado | Comandos pelo celular **não** executados neste teste: a aba foi fechada pelo usuário do PC durante a sessão |
| Microsoft Teams | sim (parado) | só `play`, `seek` | não | — | Exibido como "Microsoft Teams" |
| Spotify | — | — | — | — | Não instalado no PC de teste; não validado |

A pesquisa por voz/texto (search.v1) abriu `youtube.com/results?search_query=lofi+hip+hop+radio` no navegador padrão (Chrome) a partir do celular.
