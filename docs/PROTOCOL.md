# PC Remote Protocol 1 — aplicativos 0.1.x / 0.2.x

Contrato normativo, copiado integralmente nos dois repositórios. Transporte WSS/TLS 1.2+ na LAN, porta TCP 43127; descoberta mDNS `_pcremote._tcp.local.`. A descoberta informa endereço/nome, nunca estabelece confiança. UTF-8 JSON, limite de frame 512 KiB (inclui expansão de escape JSON de textos de 64 KiB), texto 64 KiB UTF-8, sem NUL. Versão incompatível é rejeitada. Cada mensagem tem `v:1`; IDs são UUIDs. Campos desconhecidos em comandos são rejeitados.

## Confiança e pareamento

PC gera certificado autoassinado próprio, chave protegida por Windows DPAPI (usuário atual). QR contém JSON `{v:1,host,port:43127,pcId,fingerprint,code}`; fingerprint é SHA-256 hexadecimal de todo certificado DER. A alternativa manual exige endereço, código **e fingerprint visível no PC**, não somente código. Android usa OkHttp nativo com trust manager exclusivo que aceita somente o DER fixado e válido. O hostname canônico pc-remote.local é validado pelo verificador padrão contra o SAN; um Dns exclusivo resolve esse nome para o IPv4 selecionado. Não altera trust manager ou verificador global. Código aleatório de 8 dígitos, validade 120 s, janela de pareamento explicitamente aberta, tentativas limitadas. Confirmar identidade/nome no PC antes de emitir credencial. Não se deve aceitar fingerprint obtido somente de descoberta ou de conexão não autenticada.

Credencial individual aleatória de 256 bits, armazenada por Android Keystore/AES-GCM e DPAPI no Windows. Autenticação bearer acontece somente dentro do TLS fixado. Android salva identidade apenas após aprovação. PC armazena hash SHA-256 do token. Remoção revoga hash e fecha conexão ativa. Nenhuma operação além de pareamento/hello antes de autenticar. Um PC selecionado no Android e um dispositivo controlador conectado por PC no MVP.

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
| sync.set | upload,download,paused: boolean | Preferências da conexão, inicialmente false,false,false |

Ack confirma execução, nunca apenas enfileiramento. Erros: `invalid_message`, `version_mismatch`, `unauthorized`, `pairing_expired`, `pairing_denied`, `rate_limited`, `unavailable`, `conflict`, `sensitive`, `too_large`, `clipboard_busy`, `internal` e, na extensão files.v1, `file_type`, `file_busy`, `file_integrity`, `storage`. Não ecoar conteúdo/credenciais em erros. IDs de request repetidos na conexão não executam novamente; cliente não reutiliza comandos após reconectar.

## Estado e capacidades

```json
{"v":1,"type":"state","revision":12,"audio":{"available":true,"percent":35,"muted":false,"error":null},"media":{"sessions":[{"id":"session-id","app":"player-id","title":"Título","playing":true,"capabilities":{"play":true,"pause":true,"previous":false,"next":false,"seek":true},"positionSeconds":20}]},"clipboardRevision":4,"sync":{"upload":false,"download":false,"paused":false}}
{"v":1,"type":"clipboard.event","eventId":"uuid","origin":"windows","revision":5,"text":"Olá 👋\nhttps://example.org","automatic":true}
```

Estado não inclui conteúdo do clipboard. Snapshot após hello/reconexão não transfere clipboard existente. Revision monotônica por execução do PC, não timestamp. Android aceita somente snapshots mais recentes na conexão; durante arraste mantém valor local, limita envio a 10 Hz, envia valor final e aguarda ack final antes de adotar estado remoto.

## Extensão negociada input.v1 — aplicativos 0.2.0

O Android solicita a extensão no upgrade WebSocket com `X-PC-Remote-Features: input.v1`. O header não concede acesso; autorização permanece pelo token individual dentro do TLS validado. Hello não muda. Desktop antigo ignora o header. Desktop novo só inclui `input.control` no welcome e o campo opcional `input` nos snapshots de uma conexão que negociou a extensão. Clientes antigos recebem exatamente as capacidades e formato antigos.

Snapshot negociado acrescenta `"input":{"allowed":false,"enabled":false}`. `allowed` indica a permissão local do Windows; `enabled` indica ativação nesta conexão. Ambos começam falsos. A permissão pode ser persistida pelo PC, mas a ativação nunca é persistida. Revogação, troca de conexão, suspensão, desconexão e bloqueio no PC limpam a ativação. A ação de emergência no PC invalida o epoch e encerra a conexão para impedir reativação por requests antigos; as credenciais continuam válidas. O Android desarma o PC e descarta a fila ao perder o foco da janela ou ir para segundo plano. Se o usuário não desligou o touchpad, o Android envia `input.enable` de novo ao voltar ao primeiro plano com foco, na mesma conexão autenticada; nova conexão, bloqueio ou desligamento no PC exigem ação explícita.

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

Android bloqueia texto com EXTRA_IS_SENSITIVE. Windows bloqueia ExcludeClipboardContentFromMonitorProcessing e CanUploadToCloudClipboard=0. Isso não detecta todas as senhas. Unicode e quebras preservados; rejeitar NUL e texto acima do limite. O clipboard continua só texto; imagens e PDFs do celular para o PC usam a extensão files.v1.

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
| 0.1.x / 0.2.x | versão sem protocolo 1 | outro | Rejeitado com version_mismatch |

## Ciclo de vida e extensão

Backoff com jitter 1,2,4,8,16,30 s; somente tentativa de conexão/autenticação, sem replay de ações. Heartbeat nativo WebSocket; suspensão/rede provocam nova autenticação e snapshot. Adicionar ações em enum tipada, validar autorização/capacidade antes do dispatch. Não existe endpoint shell, execução arbitrária, abertura automática de links, encaminhamento de portas ou nuvem.
