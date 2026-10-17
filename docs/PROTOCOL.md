# PC Remote Protocol 1 — aplicativos 0.1.x a 0.6.x

Contrato normativo, copiado integralmente nos dois repositórios. Transporte WSS/TLS 1.2+ na LAN, porta TCP 43127; descoberta mDNS `_pcremote._tcp.local.`. A descoberta informa endereço/nome, nunca estabelece confiança. UTF-8 JSON, limite de frame 512 KiB (inclui expansão de escape JSON de textos de 64 KiB), texto 64 KiB UTF-8, sem NUL. Versão incompatível é rejeitada. Cada mensagem tem `v:1`; IDs são UUIDs. Campos desconhecidos em comandos são rejeitados.

## Confiança e pareamento

PC gera certificado autoassinado próprio, chave protegida por Windows DPAPI (usuário atual). QR contém JSON `{v:1,host,port:43127,pcId,fingerprint,code}`; fingerprint é SHA-256 hexadecimal de todo certificado DER. A alternativa manual exige endereço, código **e fingerprint visível no PC**, não somente código. Android usa OkHttp nativo com trust manager exclusivo que aceita somente o DER fixado e válido. O hostname canônico pc-remote.local é validado pelo verificador padrão contra o SAN; um Dns exclusivo resolve esse nome para o IPv4 selecionado. Não altera trust manager ou verificador global. Código aleatório de 8 dígitos, validade 120 s, janela de pareamento explicitamente aberta, tentativas limitadas. Confirmar identidade/nome no PC antes de emitir credencial. Não se deve aceitar fingerprint obtido somente de descoberta ou de conexão não autenticada.

Credencial individual aleatória de 256 bits, armazenada por Android Keystore/AES-GCM e DPAPI no Windows. Autenticação bearer acontece somente dentro do TLS fixado. Android salva identidade apenas após aprovação. PC armazena hash SHA-256 do token. Remoção revoga hash e fecha conexão ativa. Nenhuma operação além de pareamento/hello antes de autenticar. O Android conecta a um PC por vez (desde o 0.6.1 guarda até 8 pareados) e o PC aceita um dispositivo controlador conectado por vez.

Cliente → PC:

```json
{"v":1,"type":"pair.request","id":"uuid","deviceId":"uuid","deviceName":"Meu celular","code":"12345678"}
{"v":1,"type":"hello","id":"uuid","deviceId":"uuid","token":"token-hex"}
{"v":1,"type":"request","id":"uuid","action":{"kind":"volume.set","percent":35}}
```

PC → cliente:

```json
{"v":1,"type":"pair.pending","id":"uuid"}
{"v":1,"type":"pair.result","id":"uuid","pcId":"uuid","pcName":"Meu PC","token":"token-hex"}
{"v":1,"type":"welcome","id":"uuid","pcId":"uuid","pcName":"Meu PC","capabilities":["audio","media","clipboard.text"]}
{"v":1,"type":"ack","id":"uuid"}
{"v":1,"type":"error","id":"uuid","code":"unavailable","message":"Player não permite alterar posição.","retryable":false}
```

## Ações predefinidas

| kind | Campos adicionais | Semântica |
|---|---|---|
| state.get | nenhum | Publica snapshot atual |
| volume.set | percent: inteiro 0..100 | Volume dispositivo padrão |
| volume.step | delta: -5 ou 5 | Incremento sobre estado real |
| volume.mute | muted: boolean | Define mudo explicitamente |
| media.control | sessionId: string; command: play/pause/previous/next/seek; offsetSeconds: -10 ou 10 somente para seek | Consulta capacidade antes de executar |
| clipboard.set | eventId: UUID; origin: android; text: string; automatic: boolean; baseRevision: inteiro >=0 | Atualiza clipboard, nunca cola ou executa |
| clipboard.get | nenhum | Leitura explícita; evento com automatic:false |
| sync.set | upload,download,paused: boolean | Preferências da conexão, inicialmente false,false,false. O Android 0.6 envia upload=download=«Sincronização em tempo real» e paused=false logo após o welcome e a cada mudança |

Ack confirma execução, nunca apenas enfileiramento. Erros: `invalid_message`, `version_mismatch`, `unauthorized`, `pairing_expired`, `pairing_denied`, `rate_limited`, `unavailable`, `conflict`, `sensitive`, `too_large`, `clipboard_busy`, `internal` e, nas extensões files.v1/files.v2, `file_type`, `file_busy`, `file_integrity`, `storage`. Não ecoar conteúdo/credenciais em erros. IDs de request repetidos na conexão não executam novamente; cliente não reutiliza comandos após reconectar.

## Estado e capacidades

```json
{"v":1,"type":"state","revision":12,"audio":{"available":true,"percent":35,"muted":false,"error":null},"media":{"sessions":[{"id":"session-id","app":"player-id","title":"Título","playing":true,"capabilities":{"play":true,"pause":true,"previous":false,"next":false,"seek":true},"positionSeconds":20}]},"clipboardRevision":4,"sync":{"upload":false,"download":false,"paused":false}}
{"v":1,"type":"clipboard.event","eventId":"uuid","origin":"windows","revision":5,"text":"Olá 👋\nhttps://example.org","automatic":true}
```

Estado não inclui conteúdo do clipboard. Snapshot após hello/reconexão não transfere clipboard existente. Revision monotônica por execução do PC, não timestamp. Android aceita somente snapshots mais recentes na conexão; durante arraste mantém valor local, limita envio a 10 Hz, envia valor final e aguarda ack final antes de adotar estado remoto.

Desde o desktop 0.6.0, `media.sessions` vem ordenada: primeiro a sessão que o Windows trata como atual (`GetCurrentSession`), depois as que estão tocando, depois as demais. `app` é um nome amigável quando o aplicativo é conhecido (Spotify, Google Chrome, Microsoft Edge, Firefox, Opera, Brave, VLC, Reprodutor Multimídia, Groove/Media Player); senão, o AppUserModelID. O Android 0.6 segue a primeira sessão enquanto o usuário não escolher outra. Mudar só `positionSeconds` não gera snapshot novo; depois de um `media.control` aceito o PC relê as sessões em 150, 500 e 1.200 ms e publica se algo mudou.

## Extensão negociada input.v1 — aplicativos 0.2.0

O Android solicita a extensão no upgrade WebSocket com `X-PC-Remote-Features: input.v1`. O header não concede acesso; autorização permanece pelo token individual dentro do TLS validado. Hello não muda. Desktop antigo ignora o header. Desktop novo só inclui `input.control` no welcome e o campo opcional `input` nos snapshots de uma conexão que negociou a extensão. Clientes antigos recebem exatamente as capacidades e formato antigos.

Snapshot negociado acrescenta `"input":{"allowed":false,"enabled":false}`. `allowed` indica a permissão local do Windows; `enabled` indica ativação nesta conexão. Ambos começam falsos. A permissão pode ser persistida pelo PC, mas a ativação nunca é persistida. Revogação, troca de conexão, suspensão, desconexão e bloqueio no PC limpam a ativação. A ação de emergência no PC invalida o epoch e encerra a conexão para impedir reativação por requests antigos; as credenciais continuam válidas. O Android desarma o PC e descarta a fila ao perder o foco da janela ou ir para segundo plano. Se o usuário não desligou o touchpad, o Android envia `input.enable` de novo ao voltar ao primeiro plano com foco, na mesma conexão autenticada; nova conexão, bloqueio ou desligamento no PC exigem ação explícita.

**A partir dos aplicativos 0.6.0** (sem mudança no formato): no desktop, `allowed` significa "não bloqueado no PC" e começa verdadeiro; a chave "Permitir mouse e teclado" deixou de existir. **Interromper** (painel ou bandeja) desativa a sessão, publica `allowed:false` e grava o bloqueio, que só termina com **Liberar** no PC; por isso não precisa mais encerrar a conexão. No Android 0.6, a intenção de usar o touchpad começa ligada a cada conexão: o app envia `input.enable` sozinho quando está em primeiro plano com foco e o PC permite, inclusive quando `allowed` volta a ser verdadeiro. **Desligar** no celular vale até o usuário tocar em **Ativar touchpad**, enquanto o processo existir. Uma ativação recusada ou não confirmada não é repetida automaticamente.

| kind | Campos | Limites / resultado |
|---|---|---|
| input.enable | enabled: boolean | Exige negociação e permissão do PC para ativar; false interrompe a sessão |
| pointer.move | dx,dy: inteiros | −512..512 cada; vetor não pode ser zero; movimento relativo |
| pointer.click | button: left/right; count: inteiro | 1 ou 2; right aceita somente 1; down/up no mesmo lote |
| pointer.scroll | steps: inteiro | −10..10 sem zero; positivo sobe, negativo desce; 120 unidades de roda por passo |
| keyboard.text | text: string | 1..1.024 bytes UTF-8; Unicode sem caracteres de controle; UTF-16 down/up via SendInput |
| keyboard.key | key: enum | enter,backspace,tab,escape,left,right,up,down,delete,home,end; toque completo sem tecla mantida |

Todo evento exige sessão autenticada, extensão negociada, permissão e ativação verificadas imediatamente antes da execução nativa. A API não aceita scripts, keycodes arbitrários, sequências de modificadores nem comandos de shell. O alvo é a janela em foco no Windows; só o envio explícito de texto injeta texto e não modifica clipboard. Não há captura de tela nem histórico de digitação.

Ack de movimento/clique/rolagem/teclado não carrega snapshot completo. O Android coalesce movimento, mantém no máximo uma solicitação em voo e descarta pendências ao invalidar a conexão/ativação; nunca repete automaticamente um evento com erro ou timeout. O servidor limita a frequência e conserva IDs já usados com memória limitada; a sessão continua sujeita ao timeout de uma hora e requer nova ativação após reconectar. Falha parcial de SendInput é erro sem retry, pois repetir texto poderia duplicá-lo.

SendInput pode ser bloqueado por UIPI, aplicativos elevados ou tela protegida; quantidade inserida é verificada, sem prometer diagnóstico exato da causa. Teclas físicas pressionadas podem interferir na digitação. Mais detalhes em [INPUT.md](INPUT.md).

## Clipboard e conflitos

Sem histórico persistente. Servidor serializa leituras, notificações Windows e escritas, atribuindo ordem/revision autoritativa. Eventos deduplicados por eventId (cache limitado em RAM) e escritas remotas marcadas/suprimidas no listener. Mensagem automática com baseRevision diferente da atual recebe conflict e é descartada; nenhuma repetição automática. Duas cópias concorrentes com a mesma base: primeira aceita no servidor vence, segunda é rejeitada. Notificação Windows posterior é uma nova cópia local na ordem do servidor. Envios manuais são ações explícitas e podem substituir versão atual. Relógios não arbitram conflitos. Estado antigo e eventos pendentes são descartados na desconexão. Uma atualização recebida nunca gera nova atualização automática de retorno.

Android acompanha alterações apenas em foco. Ao retomar estabelece baseline sem enviar texto preexistente. Windows acompanha WM_CLIPBOARDUPDATE, sem snapshot de conteúdo no início. Sincronização desativada por padrão em ambos; direção exige consentimento nos dois aplicativos. Aplicação automática no Android é preferência separada, somente em foco. Segundo plano não garante conexão; se houver recepção antes da suspensão, notificação genérica sem conteúdo abre app. Não há serviço permanente no MVP.

**A partir dos aplicativos 0.6.0** cada app tem uma única opção, **Sincronização em tempo real**, ligada por padrão (o Windows grava `realtime` em `settings.json`; o Android, a preferência `realtime`). O PC envia `clipboard.event` automático só com a opção ligada no PC e `download` pedido pelo celular, e aceita `clipboard.set` automático só com a opção ligada no PC e `upload` pedido pelo celular. Ações manuais (`clipboard.set` com `automatic:false`, `clipboard.get`, arquivos) não dependem da opção. No Android 0.6 com a opção ligada: o texto que chega do PC vai direto para a área de transferência do celular, também com o app em segundo plano enquanto a conexão existir (o Android limita a leitura em segundo plano, não a escrita); com o app em foco, cada cópia é enviada ao PC; ao voltar para o app, um texto copiado enquanto ele estava fora é enviado uma vez, se for diferente do último texto visto ou trocado nesta execução do app (na primeira abertura só vira referência, sem envio). O texto aplicado a partir do PC vira a referência, então não volta ao PC. "Buscar do PC" (`clipboard.get`) copia direto no celular.

Android bloqueia texto com EXTRA_IS_SENSITIVE. Windows bloqueia ExcludeClipboardContentFromMonitorProcessing e CanUploadToCloudClipboard=0. Isso não detecta todas as senhas. Unicode e quebras preservados; rejeitar NUL e texto acima do limite. Sem clipboard.v2 o clipboard é só texto; imagens e PDFs do celular para o PC usam a extensão files.v1. Imagens na área de transferência usam clipboard.v2 (aplicativos 0.6.1).

## Extensão negociada files.v1 — imagens e PDFs do celular

O Android envia o header em linhas separadas: `X-PC-Remote-Features: input.v1` e `X-PC-Remote-Features: files.v1`. Desktop até 0.3.0 lê só a primeira linha e nunca anuncia arquivos. Desktop novo junta todas as linhas e separa tokens por vírgula ou espaço (`input.v1, files.v1` numa linha também vale). O header não concede acesso. Só com files.v1 negociado o welcome acrescenta `file.transfer` e o snapshot acrescenta `"files":{"allowed":<allowFiles>,"maxBytes":52428800}`; os demais clientes recebem exatamente o formato anterior. `allowFiles` começa desligado e é persistido pelo PC; alterá-lo incrementa `revision` e publica o estado.

| kind | Campos | Regras / resultado |
|---|---|---|
| file.offer | transferId: UUID; name: string; size: inteiro; mime: string | Nesta ordem: negociado e `allowFiles`, senão `unavailable`; transferId não nulo, name 1..255 bytes UTF-8 sem controles e size ≥ 1, senão `invalid_message`; size > 52.428.800 → `too_large`; mime `image/png`, `image/jpeg` ou `application/pdf` com extensão sanitizada correspondente (png; jpg/jpeg; pdf), senão `file_type`; um ticket ou transferência por PC, senão `file_busy` (retryable); espaço livre ≥ size + 64 MiB, senão `storage`. Sucesso: `file.ticket` e depois `ack`, sem snapshot |
| file.cancel | transferId: UUID | Descarta o ticket não usado ou aborta a transferência com esse id, apagando os dados parciais. Sempre `ack`; id desconhecido não é erro; sem snapshot |

```json
{"v":1,"type":"file.ticket","id":"<id do request>","transferId":"uuid","ticket":"<64 hex minúsculos>","chunkBytes":65536,"windowBytes":1048576}
```

O ticket tem 32 bytes aleatórios, vale 30 s, é de uso único e fica vinculado a deviceId, epoch da conexão, transferId, nome sanitizado, tamanho e MIME. A comparação é em tempo constante; qualquer tentativa que cite o transferId do ticket o consome.

Socket de arquivo `wss://pc-remote.local:43127/v1/files`: mesmo listener e certificado de `/v1/ws`, `Origin` rejeitado, frames binários até 65.536 bytes e texto até 4.096 bytes, JSON estrito com `v:1`.

1. Cliente, em até 10 s: `{"v":1,"type":"file.begin","id":"uuid","transferId":"uuid","ticket":"hex"}`.
2. PC consome o ticket, confere validade, sessão com o mesmo epoch, negociação e `allowFiles`, e responde `{"v":1,"type":"file.accept","id":"<id do begin>","transferId":"uuid"}`. Problema de ticket ou sessão → `unauthorized`; `allowFiles` desligado → `unavailable`.
3. Cliente envia frames binários de 1..chunkBytes, com no máximo windowBytes enviados e ainda não confirmados.
4. PC envia `{"v":1,"type":"file.progress","transferId":"uuid","received":n}` a cada 256 KiB recebidos e quando `received == size`.
5. Cliente, após exatamente `size` bytes: `{"v":1,"type":"file.end","id":"uuid","transferId":"uuid","sha256":"<64 hex minúsculos>"}`.
6. PC confere tamanho, SHA-256 e assinatura (PNG `89 50 4E 47 0D 0A 1A 0A`, JPEG `FF D8 FF`, PDF `%PDF-` nos primeiros 1024 bytes), salva, notifica e responde `{"v":1,"type":"file.done","id":"<id do end>","transferId":"uuid","name":"<nome final>"}`, seguido de close 1000.

| code | Quando | retryable |
|---|---|---|
| `unauthorized` | ticket inválido, expirado ou usado; sessão encerrada ou revogada | false |
| `invalid_message` | JSON inválido, ordem errada, frame acima do limite | false |
| `unavailable` | files.v1 não negociado ou `allowFiles` desligado, inclusive durante o envio | false |
| `too_large` | tamanho declarado acima do limite ou mais bytes que o declarado ("Arquivo excede 50 MB") | false |
| `file_type` | MIME/extensão não permitidos ou divergentes; assinatura diferente | false |
| `file_busy` | outra transferência em andamento | true |
| `file_integrity` | menos bytes que o declarado em `file.end` ou SHA-256 diferente | true |
| `storage` | espaço insuficiente, pasta indisponível ou falha de E/S | true |
| `internal` | demais falhas, inclusive tempo esgotado | true |

Erros usam o frame `error` padrão (id da mensagem ou UUID nulo) e fecham o socket; dados parciais são sempre apagados. No clipboard, `too_large` continua "Texto excede 64 KiB". O PC aborta após 20 s sem frames, 15 min de transferência, fim ou revogação da sessão de controle (conferida a cada frame), `file.cancel`, `allowFiles` desligado ou socket fechado antes de `file.done`. Após `file.cancel` o socket fecha sem frame de erro. O Android aborta se `file.progress` não avança por 20 s ou se a conexão de controle cai.

Armazenamento no Windows: pasta Downloads (Known Folder; fallback `%USERPROFILE%\Downloads`) + `PC Remote`, criada sob demanda e recusada se não for pasta comum (link simbólico, junção ou reparse point). O nome é reduzido ao último componente; controles, `<>:"/\|?*` e marcas bidirecionais são removidos; espaços são colapsados; pontos e espaços nas pontas são removidos; a extensão fica minúscula; base vazia vira `arquivo`; nomes reservados (CON, PRN, AUX, NUL, COM0-9, LPT0-9, também com extensão) ganham `_`; a base fica com até 100 caracteres. Os bytes vão para `.<transferId>.pcremote-part` (create_new); partes antigas são apagadas antes da primeira transferência. Após a verificação: `sync_all`, marca `:Zone.Identifier` com ZoneId=3 e movimento sem substituir para `nome.ext`, `nome (1).ext` … `nome (99).ext` ou `nome <8 primeiros caracteres do transferId>.ext`.

Cliente Android: fontes são o seletor do sistema (`ACTION_OPEN_DOCUMENT` com png/jpeg/pdf e seleção múltipla) e Compartilhar (`ACTION_SEND`/`ACTION_SEND_MULTIPLE` com os três tipos), só com URIs `content://`. Nome por `OpenableColumns.DISPLAY_NAME`, tamanho por `OpenableColumns.SIZE` ou `openAssetFileDescriptor().length` (tamanho desconhecido é recusado), MIME por `ContentResolver.getType` ou pela extensão; os primeiros bytes conferem a assinatura antes de entrar na fila, e o nome passa pela mesma limpeza do PC, ganhando a extensão certa quando falta. O ticket é validado estritamente (campos exatos, hex minúsculo, `chunkBytes` 1..65.536, `windowBytes` ≥ `chunkBytes`) e só vale para a oferta em andamento; qualquer outro é liberado com `file.cancel`. Fila sequencial de até 20 arquivos; o progresso exibido é o `received` do PC. O SHA-256 é calculado durante a leitura; se o arquivo render outro número de bytes, o envio é cancelado com "O arquivo mudou durante o envio." Sem `file.transfer` no welcome, o Android não mostra nem envia nada de arquivos. Exemplos de todas as mensagens em `protocol-v1.examples.json`, com as do socket de arquivos em `fileSocket`.

Desde o desktop 0.6.0 a chave `allowFiles` não existe mais na interface: `files.allowed` é sempre `true` (o PC continua gravando `allowFiles:true` em `settings.json` para versões anteriores), e o recebimento não precisa ser liberado.

## Extensão negociada files.v2 — qualquer arquivo de até 1 GB, nos dois sentidos (aplicativos 0.6.0)

O Android 0.6 acrescenta a linha `X-PC-Remote-Features: files.v2` às de `input.v1` e `files.v1`. Desktop que não conhece files.v2 ignora o token e segue files.v1. O desktop 0.6 trata `files.v2` como files.v1 com as regras abaixo; um cliente que pede só files.v1 recebe exatamente o contrato anterior (PNG/JPG/PDF, 50 MB). Com files.v2 negociado, o welcome traz `file.transfer` **e** `file.receive`, e o snapshot traz `"files":{"allowed":true,"maxBytes":1073741824}`. `file.receive` no welcome é o sinal de que o PC aceitou files.v2.

### Celular → PC com files.v2

Mesmas mensagens e o mesmo socket de files.v1, com estas diferenças:

| Regra | files.v1 | files.v2 |
|---|---|---|
| Tamanho | 1..52.428.800 | 1..1.073.741.824 (1 GiB); acima → `too_large` ("Arquivo excede 1 GB") |
| `mime` | `image/png`, `image/jpeg`, `application/pdf`, casando com a extensão | Informativo: `tipo/subtipo` em minúsculas, até 127 bytes, só `a-z 0-9 ! # $ & ^ _ . + -` e uma `/`; formato inválido → `invalid_message`. O cliente usa `application/octet-stream` quando não sabe |
| Extensão | obrigatória e casada com o MIME | Qualquer uma, inclusive nenhuma. Recusadas com `file_type`, depois da limpeza do nome: `lnk`, `url`, `scf`, `library-ms`, `searchconnector-ms`, `settingcontent-ms`, `appref-ms`, `website` e o nome `desktop.ini` (arquivos que o Explorador interpreta só por estar na pasta) |
| Assinatura do conteúdo | conferida em `file.end` | não conferida |
| Tempo total | 15 min | maior entre 15 min e 1 s por 256 KiB declarados (1 GiB ≈ 68 min) |

Continuam iguais: nome limpo do mesmo jeito, ticket de 30 s, `chunkBytes` 65.536, `windowBytes` 1.048.576, progresso a cada 256 KiB, 20 s sem frames, SHA-256, `Zone.Identifier` com ZoneId=3 (o Windows SmartScreen avisa antes de executar um programa recebido) e movimento sem substituir. Espaço livre exigido: size + 64 MiB. Enquanto houver transferência em qualquer sentido o PC mantém um pedido de energia do Windows (`PowerCreateRequest` + `PowerSetRequest(PowerRequestSystemRequired)`, visível em `powercfg /requests` como "PC Remote: transferindo arquivo com o celular") para o PC não entrar em suspensão, e o retira ao terminar.

### PC → celular (files.v2)

O PC mantém uma fila própria (até 20 arquivos escolhidos no painel ou arrastados para a janela) e oferece um arquivo por vez, só a conexões com files.v2. A fila espera enquanto não há celular compatível conectado; nada é enviado a um celular sem `file.receive`.

1. PC → celular, no socket de controle (evento sem `id`): `{"v":1,"type":"file.push","transferId":"uuid","name":"Relatório final.pdf","size":123456,"mime":"application/pdf","ticket":"<64 hex minúsculos>"}`. `name` já vem limpo pelo PC (mesma limpeza do recebimento, 1..255 bytes UTF-8, sem controles); `size` 1..1.073.741.824; `mime` como acima (o PC deduz pela extensão ou usa `application/octet-stream`); o ticket tem 32 bytes aleatórios, vale 30 s, é de uso único e fica vinculado ao dispositivo, ao epoch da conexão e ao transferId. Outro `file.push` só depois de `file.done`, erro, recusa, cancelamento ou expiração.
2. O celular aceita sozinho: abre `/v1/files` em até 30 s e envia `{"v":1,"type":"file.fetch","id":"uuid","transferId":"uuid","ticket":"hex"}`. Para recusar (sem espaço, armazenamento indisponível), usa o controle: `{"kind":"file.decline","transferId":"uuid","code":"storage"}`, com `code` `storage` ou `unavailable`; resposta `ack`, sem snapshot. `file.cancel` com o mesmo transferId (antes ou durante o envio) cancela o push e também responde `ack`.
3. O PC consome o ticket, confere validade, sessão e epoch (problema → `unauthorized`), responde `{"v":1,"type":"file.accept","id":"<id do fetch>","transferId":"uuid"}` e envia frames binários de 1..65.536 bytes, com no máximo 1.048.576 bytes enviados e ainda não confirmados.
4. Celular → PC: `{"v":1,"type":"file.progress","transferId":"uuid","received":n}` a cada 256 KiB gravados e quando `received == size`. `received` nunca diminui nem passa do que o PC enviou.
5. PC, depois de exatamente `size` bytes: `{"v":1,"type":"file.end","id":"uuid","transferId":"uuid","sha256":"<64 hex minúsculos>"}`.
6. O celular confere tamanho e SHA-256, conclui a gravação e responde `{"v":1,"type":"file.done","id":"<id do end>","transferId":"uuid","name":"<nome final>"}`. O PC fecha com 1000.

Erros usam o frame `error` padrão nos dois sentidos e fecham o socket; o celular apaga o que gravou. Celular → PC: `storage` (sem espaço ou falha ao gravar, retryable), `file_integrity` (tamanho ou SHA-256 diferentes, retryable), `invalid_message`, `internal`. PC → celular: `unauthorized` (ticket ou sessão), `unavailable` ("Envio cancelado no PC."), `storage` (falha ao ler o arquivo no PC; o arquivo mudou de tamanho durante o envio), `internal`. O PC aborta sem progresso por 20 s, no tempo total da tabela acima, com Cancelar no painel, `file.cancel`/`file.decline`, fim ou revogação da sessão de controle; o celular aborta sem bytes por 20 s ou se a conexão de controle cai. Nada é repetido automaticamente: o item que falhou sai da fila com o motivo.

Armazenamento no Android: Android 10+ grava em `Download/PC Remote` pelo MediaStore (`IS_PENDING` durante a escrita; o próprio MediaStore escolhe `nome (1).ext` se já existir); Android 7 a 9, na pasta de downloads do próprio app (`Android/data/com.pcremote.nativeandroid/files/Download/PC Remote`), sem pedir permissão. Espaço livre exigido: size + 64 MiB. Ao concluir, uma notificação "Arquivo recebido do PC" abre o arquivo (Android 10+). O Android mantém em RAM os últimos 12 arquivos recebidos e enviados (nome, tamanho, horário).

Celular → PC no Android 0.6: o seletor (`ACTION_OPEN_DOCUMENT` com `*/*`) e o Compartilhar (`ACTION_SEND`/`ACTION_SEND_MULTIPLE` com `*/*`) aceitam qualquer arquivo; com PC files.v1 o celular só oferece PNG/JPG/PDF de até 50 MB e explica o motivo dos demais. Enquanto houver transferência e o app tiver começado em primeiro plano, um serviço em primeiro plano do tipo `dataSync` mostra o progresso e impede que o Android congele o processo; ele termina com a fila.

## Extensão negociada search.v1 — pesquisa no YouTube (aplicativos 0.6.0)

Linha `X-PC-Remote-Features: search.v1`. Só com ela o welcome traz `search.youtube`. Ação:

| kind | Campos | Regras / resultado |
|---|---|---|
| search.open | service: `youtube`; query: string | query de 1 a 200 caracteres Unicode, sem caracteres de controle e não só espaços (o PC remove os espaços das pontas), senão `invalid_message`. Não negociado → `unavailable`. Mais de 5 pesquisas em 10 s → `rate_limited`. O PC monta sozinho `https://www.youtube.com/results?search_query=<query em UTF-8, codificada como formulário>` e o abre no navegador padrão com ShellExecute; falha → `unavailable`. `ack` depois que o Windows aceitou abrir, sem snapshot |

Não existe abertura de endereço arbitrário: o celular manda só o texto da pesquisa, e a origem é fixa no PC.

## Extensão negociada info.v1 — sistema do PC (aplicativos 0.6.0)

Linha `X-PC-Remote-Features: info.v1`. Só com ela o welcome traz o campo extra `"os":"Windows 11"` (1..64 bytes UTF-8, sem controles), lido do registro do Windows (build 22000 ou maior = Windows 11). O Android mostra esse texto abaixo do nome do PC e o guarda para exibir enquanto desconectado; não o usa para decisões.

## Extensão negociada power.v1 — energia do PC (aplicativos 0.6.1)

Linha `X-PC-Remote-Features: power.v1`. Só com ela o welcome traz `power.control` e o snapshot traz o campo `power`; os demais clientes recebem exatamente o formato anterior.

```json
"power":{"allowed":true,"scheduled":null}
"power":{"allowed":true,"scheduled":{"action":"shutdown","remainingSeconds":1740}}
```

`allowed` é falso enquanto **Interromper** está ativo no PC (o mesmo bloqueio de mouse e teclado). `scheduled` é a ação agendada no PC — uma só por PC, venha de qual celular vier — com os segundos restantes (1..86.400, arredondados para cima) no momento do snapshot; o celular conta localmente a partir do recebimento. Só a contagem passar não gera snapshot; agendar, cancelar, executar e mudar `allowed` geram.

| kind | Campos | Regras / resultado |
|---|---|---|
| power.run | action: `lock`, `sleep`, `shutdown` ou `restart`; delaySeconds: inteiro 0..86.400 | `lock` só aceita 0; fora disso `invalid_message`. Sem negociação → `unavailable`. Com Interromper ativo → `unavailable`. Mais de 3 em 10 s → `rate_limited`. `sleep` num PC sem suspensão (nem S1–S3 nem Modern Standby em `GetPwrCapabilities`) → `unavailable`. Com 0, o PC cancela o agendamento existente, responde `ack` e executa 500 ms depois, para o `ack` sair antes. Com mais de 0, substitui o agendamento anterior e responde `ack` seguido de snapshot |
| power.cancel | nenhum | Cancela o agendamento, se houver. Sempre `ack`, inclusive com Interromper ativo; snapshot só quando havia agendamento |

Execução no Windows, sem nunca forçar o fechamento de programas:

- `lock`: `LockWorkStation`.
- `sleep`: `SetSuspendState(FALSE, FALSE, FALSE)` com o privilégio `SE_SHUTDOWN_NAME`. Em PCs com Modern Standby, se a chamada falhar, o PC desliga a tela com `SC_MONITORPOWER`.
- `shutdown`: `InitiateShutdownW` com prazo 0, `SHUTDOWN_POWEROFF` (mais `SHUTDOWN_HYBRID` quando a Inicialização Rápida está ligada) e motivo planejado. Programas com trabalho não salvo podem segurar o desligamento; o Windows mostra a própria tela.
- `restart`: `InitiateShutdownW` com `SHUTDOWN_RESTART | SHUTDOWN_RESTARTAPPS`.
- Uma falha depois do `ack` vira diagnóstico e notificação no PC; nada é repetido.

O agendamento fica só na memória do PC: fechar o PC Remote ou reiniciar o Windows o descarta. Ele continua se o celular desconectar. O PC notifica ao agendar ("Desligar às 23:40, pedido por <celular>") e, para prazos de 2 min ou mais, 60 s antes. Cancelam o agendamento: o painel (Visão geral), a bandeja, `power.cancel` de qualquer celular pareado, **Interromper** e a revogação do celular que agendou. No prazo, se Interromper estiver ativo, o agendamento é descartado sem executar.

## Extensão negociada clipboard.v2 — imagens na área de transferência (aplicativos 0.6.1)

Linha `X-PC-Remote-Features: clipboard.v2`. Usa o transporte de files.v2, então só vale com files.v2 também negociado; com as duas o welcome traz `clipboard.image`. As regras de consentimento são as do texto: envio automático exige **Sincronização em tempo real** ligada no PC e a direção pedida pelo celular em `sync.set`; envios manuais não dependem dela. Conteúdo marcado como sensível (Windows: `ExcludeClipboardContentFromMonitorProcessing`, `CanUploadToCloudClipboard=0`; Android: `EXTRA_IS_SENSITIVE`) nunca sai. Quando há texto na área de transferência, o texto tem prioridade e a imagem não é enviada.

Limites de imagem: PNG ou JPEG, 1..33.554.432 bytes (32 MiB), até 50.000.000 pixels (largura × altura).

### PC → celular

1. Uma cópia no Windows sem `CF_UNICODETEXT` e com imagem (formato registrado `PNG`, `CF_DIBV5` ou `CF_DIB`) vira PNG: o formato `PNG` como está, depois de conferir a assinatura; senão o DIB convertido pelo WIC. Acima dos limites, nada é enviado e o painel registra o motivo.
2. O PC anuncia com `file.push` e o campo extra `clipboard`, que só aparece em conexões com clipboard.v2: `{"v":1,"type":"file.push","transferId":"uuid","name":"Imagem do PC.png","size":123456,"mime":"image/png","ticket":"<64 hex>","clipboard":{"revision":7,"automatic":true}}`. `revision` é o `clipboardRevision` depois desta cópia. O resto do fluxo é o de files.v2 (fetch, progresso, SHA-256).
3. Pushes de clipboard não entram na fila nem no histórico de arquivos do painel; passam na frente dos arquivos que ainda esperam, mas não interrompem um arquivo em andamento. Qualquer cópia nova no PC (texto ou imagem) descarta o push de clipboard ainda não concluído: na fila, anunciado (o ticket deixa de valer) ou em transferência (erro `unavailable` no socket de arquivos).
4. `clipboard.get` sem texto e com imagem: o PC anuncia a imagem com `"automatic":false` e responde `ack` (o `ack` confirma o anúncio). Sem texto nem imagem continua `unavailable`.
5. O celular só aceita um push com `clipboard` se `mime` for `image/png` e o tamanho couber nos limites; senão recusa com `file.decline` `unavailable`. Recusa também um automático com a sincronização desligada no celular ou com `revision` menor que a última revisão de clipboard que ele viu. Grava no cache privado do app (sem MediaStore, sem a notificação "Arquivo recebido" e fora de Recentes), confere o SHA-256 e coloca na área de transferência um `content://` do próprio FileProvider (rótulo "Imagem do PC", MIME `image/png`). Guarda só a imagem mais recente. Falhas e cancelamentos de pushes de clipboard não aparecem como erro de arquivo.

### Celular → PC

1. `file.offer` com o campo extra `clipboard`, só em conexões com clipboard.v2: `{"kind":"file.offer","transferId":"uuid","name":"Imagem.png","size":123456,"mime":"image/png","clipboard":{"eventId":"uuid","automatic":true,"baseRevision":7}}`. `mime` é `image/png` ou `image/jpeg`; o celular converte outros formatos (WebP, GIF, HEIC, BMP) para PNG antes de oferecer. O nome é só informativo.
2. O PC confere, nesta ordem: negociação (sem ela o campo é desconhecido → `invalid_message`); `mime` → `file_type`; tamanho → `too_large` ("Imagem excede 32 MB"); consentimento de um automático → `unavailable`; `eventId` já visto ou, se automático, `baseRevision` diferente da atual → `conflict`. Depois segue o fluxo de files.v1/v2 (ticket, socket, progresso, SHA-256).
3. Em `file.end` o PC confere tamanho, SHA-256 e assinatura (PNG ou JPEG), decodifica com o WIC (falha → `file_type`; acima de 50 MP → `too_large`) e grava na área de transferência `CF_DIBV5` (BGRA de 32 bits) e o formato registrado `PNG` (o original, ou o JPEG convertido). Uma escrita automática exige que a área de transferência do Windows não tenha mudado desde a oferta; senão `conflict`. A escrita própria não volta ao celular. Nada é salvo em Downloads nem marcado com `Zone.Identifier`; os dados temporários são apagados. `file.done` responde com `"name":"Área de transferência"`.
4. Erros no socket de arquivos são os de files.v1/v2, mais `conflict` (retryable false).

No Android, imagens seguem as mesmas regras do texto: com foco, cada cópia de imagem é enviada; uma imagem copiada em outro app vai uma vez, ao voltar para o PC Remote; na primeira abertura só vira referência. O celular nunca reenvia um clip cujo `content://` é do próprio app. **Enviar o que copiei** envia a imagem quando o clip é uma imagem.

## Android 0.6.1 sem mudança de protocolo: vários PCs e controles de mídia fora do app

O Android guarda até 8 PCs pareados, cada um com o próprio token, identidade fixada e `deviceId`, e conecta a um por vez (o "PC atual"). Trocar de PC encerra a conexão anterior e descarta a fila e o histórico de arquivos dela, como remover o PC. Parear de novo um PC já salvo (mesmo `pcId`) substitui o registro dele. O registro único das versões anteriores é migrado para a lista.

Enquanto o PC atual estiver conectado e com uma sessão de mídia, o Android mostra uma notificação de mídia (Anterior, Tocar/Pausar e Próxima, conforme as capacidades da sessão) com uma `MediaSession` própria, que também aparece na tela de bloqueio e nos controles de mídia do sistema. Um serviço em primeiro plano do tipo `mediaPlayback` mantém a conexão enquanto a notificação existe; ele começa com o app em primeiro plano ou por um toque do usuário no widget, no bloco de Configurações rápidas ou na própria notificação. O widget e o bloco conectam ao PC atual na hora do toque, se preciso, e enviam um único `media.control` para a primeira sessão do snapshot. Os comandos são os de `media.control`; não há mensagem nova.

## Compatibilidade

| Android | Desktop | Protocolo | Resultado |
|---|---|---|---|
| 0.1.x | 0.1.x | 1 | Compatível; ações dependem das capacidades reais |
| 0.1.x | 0.2.x | 1 sem extensão | Volume, mídia e clipboard compatíveis; mouse/teclado indisponíveis |
| 0.2.x | 0.1.x | 1 sem extensão | Controles anteriores compatíveis; Android orienta atualizar PC para mouse/teclado |
| 0.2.x | 0.2.x | 1 + input.v1 | Mouse/teclado após consentimento nos dois aplicativos |
| sem files.v1 | com files.v1 | 1 (+ input.v1) | Sem capacidade nem campo de estado novos; formato anterior |
| com files.v1 | até 0.3.0 | 1 + input.v1 | Desktop lê só a primeira linha do header; arquivos indisponíveis |
| com files.v1 | com files.v1 | 1 + input.v1 + files.v1 | Imagens e PDFs após permitir no PC |
| 0.4.x / 0.5.x | 0.6.x | 1 + input.v1 + files.v1 | Formato anterior; arquivos e mouse/teclado já liberados no PC; clipboard sincroniza quando o celular liga as direções |
| 0.6.x | 0.4.x / 0.5.x | 1 + input.v1 + files.v1 | O PC ignora files.v2, search.v1 e info.v1: só PNG/JPG/PDF de até 50 MB do celular para o PC, sem PC → celular, sem pesquisa por voz no PC, sem nome do sistema; clipboard e mouse/teclado ainda pedem as chaves do PC |
| 0.6.x | 0.6.x | 1 + input.v1 + files.v1 + files.v2 + search.v1 + info.v1 | Tudo pronto após o pareamento: arquivos de até 1 GB nos dois sentidos, clipboard em tempo real, touchpad, pesquisa por voz |
| 0.6.1 | 0.6.0 | 1 + input.v1 + files.v1 + files.v2 + search.v1 + info.v1 | O PC ignora power.v1 e clipboard.v2: sem energia do PC; o clipboard sincroniza só texto. Vários PCs e controles de mídia fora do app funcionam |
| 0.6.0 | 0.6.1 | 1 + input.v1 + files.v1 + files.v2 + search.v1 + info.v1 | Formato 0.6.0, sem campos novos |
| 0.6.1 | 0.6.1 | 0.6.0 + power.v1 + clipboard.v2 | Também bloquear, suspender, desligar e reiniciar o PC (agora ou agendado) e imagens na área de transferência nos dois sentidos |
| 0.1.x / 0.2.x | versão sem protocolo 1 | outro | Rejeitado com version_mismatch |

## Ciclo de vida e extensão

Backoff com jitter 1,2,4,8,16,30 s; somente tentativa de conexão/autenticação, sem replay de ações. Heartbeat nativo WebSocket; suspensão/rede provocam nova autenticação e snapshot. Adicionar ações em enum tipada, validar autorização/capacidade antes do dispatch. Não existe endpoint shell, execução arbitrária, abertura de endereços enviados pelo celular (a pesquisa de search.v1 tem origem fixa no PC), encaminhamento de portas ou nuvem.
