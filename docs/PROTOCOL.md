# PC Remote Protocol 1 — aplicativo 0.1.0

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

Ack confirma execução, nunca apenas enfileiramento. Erros: `invalid_message`, `version_mismatch`, `unauthorized`, `pairing_expired`, `pairing_denied`, `rate_limited`, `unavailable`, `conflict`, `sensitive`, `too_large`, `clipboard_busy`, `internal`. Não ecoar conteúdo/credenciais em erros. IDs de request repetidos na conexão não executam novamente; cliente não reutiliza comandos após reconectar.

## Estado e capacidades

```json
{"v":1,"type":"state","revision":12,"audio":{"available":true,"percent":35,"muted":false,"error":null},"media":{"sessions":[{"id":"session-id","app":"player-id","title":"Título","playing":true,"capabilities":{"play":true,"pause":true,"previous":false,"next":false,"seek":true},"positionSeconds":20}]},"clipboardRevision":4,"sync":{"upload":false,"download":false,"paused":false}}
{"v":1,"type":"clipboard.event","eventId":"uuid","origin":"windows","revision":5,"text":"Olá 👋\nhttps://example.org","automatic":true}
```

Estado não inclui conteúdo do clipboard. Snapshot após hello/reconexão não transfere clipboard existente. Revision monotônica por execução do PC, não timestamp. Android aceita somente snapshots mais recentes na conexão; durante arraste mantém valor local, limita envio a 10 Hz, envia valor final e aguarda ack final antes de adotar estado remoto.

## Clipboard e conflitos

Sem histórico persistente. Servidor serializa leituras, notificações Windows e escritas, atribuindo ordem/revision autoritativa. Eventos deduplicados por eventId (cache limitado em RAM) e escritas remotas marcadas/suprimidas no listener. Mensagem automática com baseRevision diferente da atual recebe conflict e é descartada; nenhuma repetição automática. Duas cópias concorrentes com a mesma base: primeira aceita no servidor vence, segunda é rejeitada. Notificação Windows posterior é uma nova cópia local na ordem do servidor. Envios manuais são ações explícitas e podem substituir versão atual. Relógios não arbitram conflitos. Estado antigo e eventos pendentes são descartados na desconexão. Uma atualização recebida nunca gera nova atualização automática de retorno.

Android acompanha alterações apenas em foco. Ao retomar estabelece baseline sem enviar texto preexistente. Windows acompanha WM_CLIPBOARDUPDATE, sem snapshot de conteúdo no início. Sincronização desativada por padrão em ambos; direção exige consentimento nos dois aplicativos. Aplicação automática no Android é preferência separada, somente em foco. Segundo plano não garante conexão; se houver recepção antes da suspensão, notificação genérica sem conteúdo abre app. Não há serviço permanente no MVP.

Android bloqueia texto com EXTRA_IS_SENSITIVE. Windows bloqueia ExcludeClipboardContentFromMonitorProcessing e CanUploadToCloudClipboard=0. Isso não detecta todas as senhas. Unicode e quebras preservados; rejeitar NUL e texto acima do limite. Imagens/arquivos requerem capacidades e versão futuras; não estão neste protocolo.

## Compatibilidade

| Android | Desktop | Protocolo | Resultado |
|---|---|---|---|
| 0.1.x | 0.1.x | 1 | Compatível; ações dependem das capacidades reais |
| 0.1.x | versão sem protocolo 1 | outro | Rejeitado com version_mismatch |

## Ciclo de vida e extensão

Backoff com jitter 1,2,4,8,16,30 s; somente tentativa de conexão/autenticação, sem replay de ações. Heartbeat nativo WebSocket; suspensão/rede provocam nova autenticação e snapshot. Adicionar ações em enum tipada, validar autorização/capacidade antes do dispatch. Não existe endpoint shell, execução arbitrária, abertura automática de links, encaminhamento de portas ou nuvem.
