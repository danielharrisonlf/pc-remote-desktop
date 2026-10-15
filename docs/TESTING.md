# Validação e critérios de aceite

O código contém integrações nativas reais. Testes unitários não comprovam compatibilidade de cada player, permissões do Android nem continuidade durante suspensão. Este registro distingue execução automatizada, execução no Windows deste ambiente e validações pendentes em Android físico.

## Atualização 0.2.1 — endereço LAN do convite, 15/10/2026

- Reproduzido no Windows deste ambiente: a ordenação lexicográfica colocava o endereço de `vEthernet (Default Switch)` antes do IPv4 Wi-Fi que possui rota padrão. O QR usava esse primeiro endereço.
- A enumeração agora promove `local_ip_address::local_ip()` somente quando esse endereço consta entre os IPv4 válidos enumerados. A implementação Windows da dependência usa `GetIpForwardTable` e `GetAdaptersAddresses`, sem enviar tráfego. Demais endereços, deduplicação e fallback determinístico são preservados.
- **22 testes Rust passaram**, com os dois novos testes de múltiplas interfaces e fallback. Os três smokes antigos que alteram volume/clipboard/cursor não foram executados nessa rodada. O novo smoke de rede também é ignorado por padrão.
- O smoke `default_route_qr_invitation_native_smoke` foi executado separadamente no Windows real e passou: o payload do convite gerado por um Core efêmero usou o endereço da interface com rota padrão. Não grava credenciais nem altera rede, clipboard, áudio ou cursor.
- **2 testes Vitest passaram** e TypeScript passou. Revisão independente não encontrou bloqueio no código alterado.
- `npm run tauri build -- --target x86_64-pc-windows-msvc` concluiu Vite, Rust release e NSIS. Instalador x64: 3.470.366 bytes, sem Authenticode. A tentativa inicial de teste pelo diretório pai escolheu o toolchain GNU e foi interrompida; a validação final utilizou MSVC no diretório do projeto com target explícito.

Rede com múltiplas rotas padrão/VPN pode continuar exigindo endereço manual alcançável pelo celular. Não foi certificada descoberta em toda combinação de interfaces. Sem IPv4 válido, o fallback anterior de loopback no convite permanece; é necessário conectar o PC a uma rede antes de parear. O instalador atualizado não foi instalado por cima do processo do usuário durante a validação.

## Validação da versão 0.2.0 — touchpad e teclado

| Verificação | Resultado |
|---|---|
| Android | TypeScript estrito passou; 18 testes Jest passaram, incluindo movimento acumulado/fracionário, descarte de fila, Unicode/limites e ACK tardio após desativação |
| Desktop | TypeScript, Vite e 2 testes Vitest passaram |
| Rust | 20 testes passaram; 3 smokes nativos ignorados por padrão. Migração de preferências não concede entrada; execução após desativar/revogar é rejeitada |
| Protocolo real TLS/WebSocket | Cliente legado continua sem campos/capacidades extras; cliente input.v1 recebe estado opcional; comandos sem permissão são rejeitados |
| Entrada nativa real no Windows | Smoke SendInput moveu o cursor e restaurou a posição anterior |
| Windows + emulador API36 | Pareamento, capacidade input.v1, ativação explícita e desativação ao pressionar Home confirmados |
| Inicialização com credencial salva | Recepção de welcome/state após iniciar o APK sem Metro confirmada; conexão espera os listeners JS para não perder o estado inicial |
| Ciclo de vida Android nativo | Pause/destroy enviam disable sem depender do JS; requests de entrada são bloqueados até a retomada ser reconhecida pelo store |
| Artefatos | APKs ARM64/x86_64 release e instalador Windows x64 gerados; assinatura de avaliação dos APKs verificada |

A revisão independente apontou o risco de entrada durante suspensão do JS. O integrador corrigiu o bloqueio nativo, separou a confirmação de retomada do clipboard e validou a inicialização com credencial salva. Agentes de frontend e testes atingiram limite de uso; os ajustes finais foram concluídos pelo integrador.

Ainda pendentes: validação em celular físico; precisão/latência em Wi-Fi real; digitação e teclas em vários aplicativos/IMEs; clique duplo, rolagem e recusas por UIPI/UAC em hardware do usuário. A tentativa de digitação em janela descartável não é tratada como sucesso sem leitura de resultado. O escopo não inclui transmissão de tela, arrastar com botão mantido ou atalhos com modificadores. Builds permanecem de avaliação, sem assinatura de produção.

## Resultados da versão 0.1.0 — 14/10/2026

| Verificação | Resultado |
|---|---|
| Android TypeScript estrito | Passou |
| Android Jest | 10 testes passaram: protocolo/exemplos, Unicode/limites/escapes, leitura manual, dedup, revisão, recepção em segundo plano sem autoapply, slider/frequência/valor final e snapshots antigos |
| Desktop TypeScript e Vite | Passaram |
| Desktop Vitest | 2 testes passaram |
| Rust/MSVC | 15 testes passaram; 2 smokes nativos são ignorados por padrão e foram executados separadamente |
| Transporte integrado real | TLS + WebSocket: pareamento/aprovação, hello, raiz TLS incorreta, cliente não autorizado, ID repetido, revogação e limpeza de pedido desconectado passaram |
| Retomada e filas | Testes de invalidação da sessão após pausa e autorização revalidada na execução de mídia passaram; suspensão física ainda pendente |
| Core Audio Windows real | Definir/ler volume e inverter mudo passou; estado anterior restaurado |
| Clipboard Windows real | Escrever/ler Unicode e quebras passou; texto anterior restaurado em RAM, sem registro do conteúdo |
| APKs release | ARM64 e x86_64 gerados, com JavaScript embutido; assinatura de avaliação verificada com apksigner |
| Instalador Windows | NSIS x64 gerado; configuração por usuário |
| Windows + emulador Android API36 | Pareamento manual com fingerprint e aprovação, estado real de volume, ±5%, mudo e transferência manual nas duas direções com texto Unicode confirmados |
| Compartilhar Android | ACTION_SEND text/plain recebido com app aberto e encerrado no emulador; envio exige ação explícita com sync desligada |
| Janela desktop fechada | Evento de fechamento ocultou o painel; Android continuou recebendo confirmação e estado de volume |
| Mídia sem sessão | UI desabilitou os controles e apresentou explicação; consulta GSMTC nativa executada |

Os testes de controle e texto no emulador usaram o APK x86_64 release, sem Metro. O texto de teste era uma amostra conhecida com acentos, emoji, link e três linhas. Tentativa de executar a variante ARM64 por tradução no emulador x86_64 encontrou uma limitação de SoLoader; o APK ARM64 contém as bibliotecas ARM64 e está entregue para instalação em hardware dessa arquitetura, ainda não testado fisicamente.

## Pendências reais

- Android físico: scanner QR, comportamento específico de fabricante/Doze, notificações autorizadas e leitura de clipboard em foco/segundo plano.
- Rede Wi-Fi física: descoberta mDNS, mudança de interface/roteador/IP, regras de firewall e isolamento de clientes.
- Players reais com sessões simultâneas: play/pause/faixas, seleção e seek ±10s de cada versão. Nenhuma compatibilidade comercial universal é alegada.
- Troca/desconexão de saída de áudio e suspensão/retomada física do PC.
- Sincronização automática com ambos os sentidos habilitados em hardware e flags de sensibilidade reais. Regras, dedup e conflito foram automatizados; o fluxo manual foi integrado.
- Assinatura de produção: keystore próprio Android e certificado Authenticode Windows. APK usa assinatura debug de avaliação; instalador não possui Authenticode.

A revisão independente encontrou defeitos de owner HWND no clipboard, revisão/eco, autorização/revogação, cleanup e snapshots do slider. Foram corrigidos e validados pelo integrador. Agentes auxiliares atingiram limite de uso; a integração final e as builds foram concluídas pelo agente raiz.

## Comandos reproduzíveis

Android: `npm ci`, `npm run typecheck`, `npm test -- --runInBand`, `android/gradlew.bat assembleRelease`. Desktop: `npm ci`, `npm run typecheck`, `npm test`, `npm run build`, `cargo +stable-x86_64-pc-windows-msvc test --manifest-path src-tauri/Cargo.toml`, `npm run tauri build -- --target x86_64-pc-windows-msvc`.

As cópias de `docs/PROTOCOL.md` e `docs/protocol-v1.examples.json` devem ser idênticas entre repos. Os exemplos contêm credenciais sintéticas sem acesso a nenhuma instalação. O texto de limite é 64 KiB UTF-8; o limite de transporte é 512 KiB para acomodar escapes JSON. Teste também 65.536 aspas e controles Unicode, além de emojis multibyte.

## Checklist integrado em LAN

| Critério | Procedimento/evidência necessária |
|---|---|
| Pareamento QR/manual | Convite no PC, scanner ou fingerprint+código, aprovação; testar negar e expirar |
| Identidade TLS | Alterar um caractere do fingerprint; conexão deve falhar antes de enviar código/token |
| Autorização/revogação | Request antes de hello rejeitado; token inválido rejeitado; revogação encerra conexão e impede novas ações |
| Volume/mudo | Slider e ±5/mudo; alterar volume no Windows; trocar saída, desconectar/reconectar dispositivo |
| Arraste | Arrastar rápido, soltar em valor final; verificar frequência ≤10Hz e nenhuma volta para snapshot antigo |
| Mídia | Player com sessão GSMTC; seleção entre duas sessões; play/pause/faixas; capacidade negativa desabilita e servidor rejeita |
| Vídeo | Seek ±10s somente com flag de posição; confirmar resultado do player e separar faixa de posição |
| Texto manual | Enviar Android→PC e solicitar PC→Android sem sync; conferir acentos, emojis e quebras |
| Compartilhar | ACTION_SEND text/plain com aplicativo fechado e aberto; desligado permanece preview e envio explícito |
| Sensível/limite | Flag EXTRA_IS_SENSITIVE; Windows formatos de exclusão; >64KiB e NUL rejeitados sem registrar texto |
| Sync/foco | Duas direções opt-in; baseline em abrir/retomar/habilitar não envia conteúdo preexistente |
| Loops | Aplicar texto remoto e observar não repetir evento; dedup eventId com cache limitado |
| Conflito | Duas cópias automáticas com mesma baseRevision: primeira do servidor aceita; outra conflict; sem retry automático |
| Segundo plano | Android10+ sem ler outros apps; aviso genérico se autorizado e evento recebido antes da suspensão; sem autoapply tardio |
| Reconexão | Wi-Fi desligar/ligar, mudar rede, PC dormir/retomar; atualizar estado, não repetir mídia ou clipboard antigos |
| Bandeja | Fechar janela, operar no Android e reabrir pela bandeja; Encerrar remove serviço |
| Distribuição | APK release instalado sem Metro, cabo ou debug tools; instalador Windows por usuário |

## Estado de hardware

Nenhum Android físico estava conectado ao início da execução. A validação em um emulador, se executada, será identificada separadamente e não certificará comportamento de fabricantes, Doze, descoberta em Wi-Fi real ou compatibilidade de players em hardware do usuário.

## Memória e publicação

Semantica não estava disponível nesta sessão. Decisões e evidências ficam versionadas nestes documentos; não foi alegada persistência em memória externa. Os repositórios Git locais são independentes, sem monorepo. Repositórios GitHub foram criados privados; a publicação de código é registrada no relatório final da entrega.
