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
