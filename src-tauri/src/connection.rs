use crate::{
    core::{Core, Outgoing},
    energy,
    files::{self, Accepted, FileKind},
    outbox::Fetched,
    pairing::ApprovalOutcome,
    protocol::{self, Action, ClientMessage, DeclineCode, Features, FileClientMessage, Wording},
};
use futures_util::{SinkExt, StreamExt};
use mdns_sd::{DaemonEvent, IfKind, ServiceDaemon, ServiceInfo};
use rustls::{
    pki_types::{CertificateDer, PrivateKeyDer},
    ServerConfig,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    net::TcpListener,
    sync::{mpsc, Semaphore},
    time::{timeout, Instant},
};
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::{
    accept_hdr_async_with_config,
    tungstenite::{
        handshake::server::{ErrorResponse, Request, Response},
        protocol::{frame::coding::CloseCode, CloseFrame, Role, WebSocketConfig},
        Bytes, Error as WsError, Message, Utf8Bytes,
    },
    WebSocketStream,
};
use uuid::Uuid;

type ServerSocket = WebSocketStream<tokio_rustls::server::TlsStream<tokio::net::TcpStream>>;

/// The server pings every 30 s and phones answer on their own; three missed
/// rounds mean the phone left the network without closing the socket.
const CONTROL_SILENCE_LIMIT: Duration = Duration::from_secs(90);
/// A peer that stops reading must not hold a connection slot forever.
const SEND_TIMEOUT: Duration = Duration::from_secs(30);
/// Recent request ids kept to refuse a repeated one without growing forever.
const SEEN_IDS: usize = 4096;
const FILE_BEGIN_TIMEOUT: Duration = Duration::from_secs(10);
const FILE_IDLE_TIMEOUT: Duration = Duration::from_secs(20);
const FILE_PROGRESS_STEP: u64 = 256 * 1024;
const FILE_HEAD_BYTES: usize = 1024;
/// Chunks queued for the blocking writer; matches the 1 MiB client window.
const FILE_QUEUE_CHUNKS: usize = protocol::FILE_WINDOW_BYTES / protocol::FILE_CHUNK_BYTES;

/// rustls is built with aws-lc-rs (its default, used by this server since 0.1) and ring
/// (enabled by the updater), so the process-wide provider must be chosen explicitly.
/// aws-lc-rs stays the provider for the local server and the updater requests.
/// Installing again is a no-op.
pub fn install_crypto_provider() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

pub fn tls_config(certificate_pem: &str, key_pem: &str) -> Result<(ServerConfig, String), String> {
    install_crypto_provider();
    let certs: Vec<CertificateDer<'static>> =
        rustls_pemfile::certs(&mut Cursor::new(certificate_pem))
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
    let cert = certs.first().ok_or("certificado ausente")?;
    let fingerprint = hex::encode(Sha256::digest(cert.as_ref()));
    let key: PrivateKeyDer<'static> = rustls_pemfile::private_key(&mut Cursor::new(key_pem))
        .map_err(|e| e.to_string())?
        .ok_or("chave ausente")?;
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| e.to_string())?;
    Ok((config, fingerprint))
}

pub async fn serve(core: Arc<Core>, config: ServerConfig) -> Result<(), String> {
    let listener = TcpListener::bind(("0.0.0.0", protocol::PORT))
        .await
        .map_err(|e| e.to_string())?;
    let mdns = match ServiceDaemon::new() {
        Ok(daemon) => Some(daemon),
        Err(_) => {
            core.diagnostic("Descoberta mDNS indisponível. Use o endereço manual.");
            None
        }
    };
    let status = core.status();
    match ServiceInfo::new(
        "_pcremote._tcp.local.",
        &status.pc_name,
        &format!("{}.local.", status.pc_id),
        (),
        protocol::PORT,
        &[("id", status.pc_id.to_string().as_str()), ("v", "1")][..],
    ) {
        Ok(info) => {
            if let Some(mdns) = &mdns {
                // The TLS listener is IPv4 only. Auto-addressing still follows IP changes.
                if mdns.disable_interface(IfKind::IPv6).is_err() {
                    core.diagnostic("Descoberta mDNS: não foi possível limitar o anúncio a IPv4.");
                } else {
                    match mdns.monitor() {
                        Ok(events) => {
                            let monitor_core = core.clone();
                            std::thread::spawn(move || {
                                while let Ok(event) = events.recv() {
                                    if matches!(event, DaemonEvent::Error(_)) {
                                        monitor_core.diagnostic(
                                            "Descoberta mDNS: falha ao anunciar o serviço.",
                                        );
                                    }
                                }
                            });
                        }
                        Err(_) => core.diagnostic("Descoberta mDNS: monitor indisponível."),
                    }
                    if mdns.register(info.enable_addr_auto()).is_err() {
                        core.diagnostic("Descoberta mDNS: falha ao registrar o serviço.");
                    }
                }
            }
        }
        Err(_) => core.diagnostic("Descoberta mDNS: serviço local inválido."),
    }
    let acceptor = TlsAcceptor::from(Arc::new(config));
    core.set_service_running(true);
    let slots = Arc::new(Semaphore::new(16));
    loop {
        // A peer resetting before accept, or a brief handle shortage, must not
        // take the service down until the app restarts.
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let _ = stream.set_nodelay(true);
        let Ok(permit) = slots.clone().try_acquire_owned() else {
            continue;
        };
        let core = core.clone();
        let acceptor = acceptor.clone();
        tokio::spawn(async move {
            let _permit = permit;
            // No overall cap: the control socket lives as long as the phone answers
            // pings, and file sockets keep their own idle and total deadlines.
            if let Ok(Ok(tls)) = timeout(Duration::from_secs(8), acceptor.accept(stream)).await {
                let _ = handle(core, tls).await;
            }
        });
    }
}

async fn send(ws: &mut ServerSocket, v: Value) -> Result<(), String> {
    send_message(ws, Message::Text(v.to_string().into())).await
}
async fn send_message(ws: &mut ServerSocket, message: Message) -> Result<(), String> {
    timeout(SEND_TIMEOUT, ws.send(message))
        .await
        .map_err(|_| "send timeout".to_string())?
        .map_err(|e| e.to_string())
}
async fn error(ws: &mut ServerSocket, id: Uuid, code: &str) -> Result<(), String> {
    reject(ws, id, code, Wording::Control).await
}
/// Error frame; `wording` selects the file-transfer limit for shared codes.
async fn reject(
    ws: &mut ServerSocket,
    id: Uuid,
    code: &str,
    wording: Wording,
) -> Result<(), String> {
    let (message, retryable) = protocol::error_text(code, wording);
    send(ws, protocol::wire_error(id, code, message, retryable)).await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    Control,
    Files,
}

fn route(req: &Request) -> Option<(Route, Features)> {
    if req.headers().contains_key("origin") {
        return None;
    }
    let route = match req.uri().path() {
        "/v1/ws" => Route::Control,
        "/v1/files" => Route::Files,
        _ => return None,
    };
    let values = req.headers().get_all(protocol::FEATURES_HEADER);
    Some((route, Features::parse(values.iter().map(|v| v.as_bytes()))))
}

fn file_socket_config() -> WebSocketConfig {
    let mut config = WebSocketConfig::default();
    config.max_message_size = Some(protocol::FILE_CHUNK_BYTES);
    config.max_frame_size = Some(protocol::FILE_CHUNK_BYTES);
    config
}

pub(crate) async fn handle(
    core: Arc<Core>,
    tls: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
) -> Result<(), String> {
    let mut config = WebSocketConfig::default();
    config.max_message_size = Some(protocol::MAX_FRAME);
    config.max_frame_size = Some(protocol::MAX_FRAME);
    let mut routed = None;
    let ws = timeout(
        Duration::from_secs(8),
        accept_hdr_async_with_config(
            tls,
            |req: &Request, response: Response| -> Result<Response, ErrorResponse> {
                routed = route(req);
                if routed.is_some() {
                    Ok(response)
                } else {
                    Err(tungstenite_forbidden())
                }
            },
            Some(config),
        ),
    )
    .await
    .map_err(|_| "WebSocket upgrade timeout")?
    .map_err(|e| e.to_string())?;
    match routed {
        Some((Route::Control, features)) => control(core, ws, features).await,
        Some((Route::Files, _)) => {
            // RFC 6455 clients wait for the 101 response before sending frames, so
            // nothing is buffered yet. Re-wrap the stream with the tighter limits.
            let ws = WebSocketStream::from_raw_socket(
                ws.into_inner(),
                Role::Server,
                Some(file_socket_config()),
            )
            .await;
            file_socket(core, ws).await
        }
        None => Ok(()),
    }
}

async fn control(core: Arc<Core>, mut ws: ServerSocket, features: Features) -> Result<(), String> {
    struct Cleanup {
        core: Arc<Core>,
        device: Option<Uuid>,
        epoch: u64,
    }
    impl Drop for Cleanup {
        fn drop(&mut self) {
            if let Some(id) = self.device {
                self.core.disconnect(id, self.epoch);
            }
        }
    }
    let mut cleanup = Cleanup {
        core: core.clone(),
        device: None,
        epoch: 0,
    };
    let mut device: Option<Uuid> = None;
    let mut seen = std::collections::HashSet::<Uuid>::new();
    let mut seen_order = std::collections::VecDeque::<Uuid>::new();
    let mut last_rx = tokio::time::Instant::now();
    let mut events = core.events.subscribe();
    let connected_at = tokio::time::Instant::now();
    let first_deadline = connected_at + Duration::from_secs(10);
    let unauth_deadline = connected_at + Duration::from_secs(180);
    let mut first_valid_message = false;
    let mut heartbeat = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_secs(30),
        Duration::from_secs(30),
    );
    loop {
        tokio::select! {
            _=tokio::time::sleep_until(first_deadline),if device.is_none()&&!first_valid_message=>break,
            _=tokio::time::sleep_until(unauth_deadline),if device.is_none()=>break,
            _=heartbeat.tick()=>{
                if last_rx.elapsed()>=CONTROL_SILENCE_LIMIT{break;}
                send_message(&mut ws,Message::Ping(Vec::new().into())).await?;
            }
            incoming=ws.next()=>{
                core.check_resume();
                let Some(Ok(msg))=incoming else {break};
                last_rx=tokio::time::Instant::now();
                if msg.is_close(){break;}
                if msg.is_ping(){send_message(&mut ws,Message::Pong(msg.into_data())).await?;continue;}
                let Message::Text(raw)=msg else {continue};
                if raw.len()>protocol::MAX_FRAME{let _=error(&mut ws,Uuid::nil(),"too_large").await;break;}
                let parsed=serde_json::from_str::<ClientMessage>(&raw);
                let msg=match parsed {Ok(m)=>m,Err(_)=>{error(&mut ws,Uuid::nil(),"invalid_message").await?;continue}};
                let id=msg.id();
                if msg.version()!=1{error(&mut ws,id,"version_mismatch").await?;continue;}
                if id.is_nil(){error(&mut ws,id,"invalid_message").await?;break;}
                if !seen.insert(id){error(&mut ws,id,"invalid_message").await?;continue;}
                // Every pointer move has its own id; a hard cap ended long mouse sessions.
                seen_order.push_back(id);
                if seen_order.len()>SEEN_IDS{if let Some(old)=seen_order.pop_front(){seen.remove(&old);}}
                match msg {
                    ClientMessage::PairRequest {device_id,device_name,code,..} if device.is_none()=>{
                        match core.pair_request(id,device_id,device_name.clone(),code){
                            Ok(receiver)=>{
                                first_valid_message=true;
                                struct PendingCleanup {core:Arc<Core>,id:Uuid}
                                impl Drop for PendingCleanup {fn drop(&mut self){self.core.cancel_pending(self.id);}}
                                let _pending=PendingCleanup{core:core.clone(),id};
                                send(&mut ws,json!({"v":1,"type":"pair.pending","id":id})).await?;
                                let mut receiver=receiver;
                                let mut deadline=core.expire_pending(id)
                                    .map(tokio::time::Instant::from_std)
                                    .unwrap_or_else(tokio::time::Instant::now);
                                let outcome=loop {tokio::select!{
                                    answer=&mut receiver=>break answer.unwrap_or(ApprovalOutcome::Invalidated),
                                    _=tokio::time::sleep_until(deadline)=>{
                                        deadline=core.expire_pending(id)
                                            .map(tokio::time::Instant::from_std)
                                            .unwrap_or_else(||tokio::time::Instant::now()+Duration::from_millis(10));
                                    },
                                    incoming=ws.next()=>match incoming{
                                        Some(Ok(Message::Ping(payload)))=>send_message(&mut ws,Message::Pong(payload)).await?,
                                        Some(Ok(Message::Pong(_)))=>{},
                                        _=>return Ok(())
                                    }
                                }};
                                // No heartbeat runs while waiting for the user; do not count that wait as silence.
                                last_rx=tokio::time::Instant::now();
                                match outcome {
                                    ApprovalOutcome::Approved(grant) => match core.pair_complete(grant) {
                                        Ok((pc_id,token))=>{let status=core.status();send(&mut ws,json!({"v":1,"type":"pair.result","id":id,"pcId":pc_id,"pcName":status.pc_name,"token":token})).await?},
                                        Err(code)=>error(&mut ws,id,&code).await?,
                                    },
                                    ApprovalOutcome::Denied => error(&mut ws,id,"pairing_denied").await?,
                                    ApprovalOutcome::Expired => error(&mut ws,id,"pairing_timeout").await?,
                                    ApprovalOutcome::Invalidated => error(&mut ws,id,"pairing_invalidated").await?,
                                }
                            },Err(code)=>error(&mut ws,id,code).await?
                        }
                    }
                    ClientMessage::Hello {device_id,token,..} if device.is_none()=>{
                        let epoch = match core.connect_authenticated(device_id,&token,features) { Ok(epoch) => epoch, Err(code) => {error(&mut ws,id,code).await?;break;} };
                        events = core.events.subscribe();
                        first_valid_message=true;device=Some(device_id);cleanup.device=device;cleanup.epoch=epoch;let status=core.status();
                        let mut capabilities = vec!["audio", "media", "clipboard.text"];
                        if features.input { capabilities.push("input.control"); }
                        if features.input && features.numpad { capabilities.push("input.numpad"); }
                        if features.input && features.presentation { capabilities.push("presentation.control"); }
                        if features.input && features.deck { capabilities.push("deck.control"); }
                        if features.wol { capabilities.push("wol.target"); }
                        if features.files { capabilities.push("file.transfer"); }
                        if features.files_v2 { capabilities.push("file.receive"); }
                        if features.search { capabilities.push("search.youtube"); }
                        if features.power { capabilities.push("power.control"); }
                        if features.clipboard_image { capabilities.push("clipboard.image"); }
                        let mut welcome = json!({"v":1,"type":"welcome","id":id,"pcId":status.pc_id,"pcName":status.pc_name,"capabilities":capabilities});
                        if features.info { welcome["os"] = json!(crate::system::os_name()); }
                        if features.wol { welcome["macs"] = json!(crate::system::mac_addresses()); }
                        send(&mut ws,welcome).await?;
                        send(&mut ws,serde_json::to_value(core.snapshot()).map_err(|e|e.to_string())?).await?;
                        // A push queued while no phone was connected goes out after the first snapshot.
                        core.outgoing_connected();
                    }
                    ClientMessage::Request {action,..} if device.is_some()=>{
                        if !core.is_authorized_session(device.unwrap(),cleanup.epoch){error(&mut ws,id,"unauthorized").await?;break;}
                        let file = action.is_file();
                        let files_v2 = core.files_v2(device.unwrap(),cleanup.epoch);
                        let wording = match (file, files_v2) { (false, _) => Wording::Control, _ if action.is_clipboard_image() => Wording::Image, (true, false) => Wording::FilesV1, (true, true) => Wording::FilesV2 };
                        if file {if let Err(code)=core.file_gate(device.unwrap(),cleanup.epoch){reject(&mut ws,id,code,wording).await?;continue;}}
                        if matches!(&action, Action::FileDecline { .. }) && !files_v2 {reject(&mut ws,id,"unavailable",wording).await?;continue;}
                        if matches!(&action, Action::SearchOpen { .. }) && !features.search {error(&mut ws,id,"unavailable").await?;continue;}
                        if action.is_power() && !features.power {error(&mut ws,id,"unavailable").await?;continue;}
                        let negotiated = protocol::Features { files_v2, ..features };
                        if let Err(code)=protocol::validate_action_for(&action,&negotiated){reject(&mut ws,id,code,wording).await?;continue;}
                        // Power changes publish their own snapshot, which then follows the ack.
                        let send_snapshot = !(action.is_input() || file || action.is_power() || matches!(&action, Action::SearchOpen { .. })) || matches!(&action, Action::InputEnable { .. });
                        let result=dispatch(core.clone(),device.unwrap(),cleanup.epoch,id,action).await;
                        // The socket was not read while the action ran; that is not the phone's silence.
                        last_rx=tokio::time::Instant::now();
                        match result {Ok(reply)=>{if let Some(reply)=reply {send(&mut ws,reply).await?;}if send_snapshot {send(&mut ws,serde_json::to_value(core.snapshot()).map_err(|e|e.to_string())?).await?;}send(&mut ws,protocol::ack(id)).await?},Err(code)=>reject(&mut ws,id,&code,wording).await?}
                    }
                    _=>{error(&mut ws,id,"unauthorized").await?;break;}
                }
            }
            outgoing=events.recv(),if device.is_some()=>{
                match outgoing {Ok(Outgoing::Wire{device_id,message}) if Some(device_id)==device=>send(&mut ws,message).await?,Ok(Outgoing::Close{device_id}) if Some(device_id)==device=>break,_=>{}}
            }
        }
    }
    let _ = ws.close(None).await;
    let _ = timeout(Duration::from_millis(250), async {
        while let Some(Ok(message)) = ws.next().await {
            if message.is_close() {
                break;
            }
        }
    })
    .await;
    Ok(())
}

fn tungstenite_forbidden() -> ErrorResponse {
    tokio_tungstenite::tungstenite::http::Response::builder()
        .status(403)
        .body(Some("Forbidden".into()))
        .unwrap()
}

/// What a finished upload became: a file in Downloads or a clipboard.v2 image in memory.
enum Received {
    Saved(String),
    Image(Vec<u8>),
}

/// Why a transfer stopped before `file.done`. Partial data is always deleted.
enum FileAbort {
    Reply(Uuid, &'static str),
    Storage(Uuid, String),
    Timeout,
    Cancelled,
    Closed,
}

impl FileAbort {
    fn from_check(id: Uuid, code: &'static str) -> Self {
        if code == "cancelled" {
            Self::Cancelled
        } else {
            Self::Reply(id, code)
        }
    }
    fn reply(&self) -> Option<(Uuid, &'static str)> {
        match self {
            Self::Reply(id, code) => Some((*id, code)),
            Self::Storage(id, _) => Some((*id, "storage")),
            Self::Timeout => Some((Uuid::nil(), "internal")),
            Self::Cancelled | Self::Closed => None,
        }
    }
    fn diagnostic(&self, image: bool) -> String {
        if image {
            return match self {
                Self::Cancelled => "Imagem do celular cancelada no celular.".into(),
                Self::Reply(_, "file_type") => {
                    "Imagem do celular descartada: formato não reconhecido.".into()
                }
                _ => "Imagem do celular descartada antes de chegar à área de transferência.".into(),
            };
        }
        let reason = match self {
            Self::Reply(_, "unauthorized") => "a sessão do celular terminou",
            Self::Reply(_, "unavailable") => "recebimento desativado no PC",
            Self::Reply(_, "too_large") => "mais dados que o tamanho informado",
            Self::Reply(_, "file_integrity") => "falha de integridade",
            Self::Reply(_, "file_type") => "o conteúdo não corresponde ao tipo",
            Self::Reply(..) => "mensagem inválida",
            Self::Storage(_, detail) => {
                return format!(
                    "Não foi possível salvar o arquivo do celular em Downloads\\{}: {detail}",
                    files::FOLDER_NAME
                )
            }
            Self::Timeout => "tempo esgotado",
            Self::Cancelled => return "Recebimento de arquivo cancelado no celular.".into(),
            Self::Closed => "o celular encerrou o envio",
        };
        format!("Arquivo do celular descartado: {reason}.")
    }
}

fn parse_file_message(raw: &str) -> Result<FileClientMessage, (Uuid, &'static str)> {
    if raw.len() > protocol::FILE_TEXT_LIMIT {
        return Err((Uuid::nil(), "invalid_message"));
    }
    let message = serde_json::from_str::<FileClientMessage>(raw)
        .map_err(|_| (Uuid::nil(), "invalid_message"))?;
    if message.version() != 1 {
        return Err((message.id(), "version_mismatch"));
    }
    // `file.progress` has no id, and an error may name the nil UUID (PROTOCOL.md).
    let id_optional = matches!(
        message,
        FileClientMessage::Progress { .. } | FileClientMessage::Error { .. }
    );
    if message.id().is_nil() && !id_optional {
        return Err((Uuid::nil(), "invalid_message"));
    }
    Ok(message)
}

async fn close_file_socket(ws: &mut ServerSocket, reply: Option<(Uuid, &str)>, done: bool) {
    close_file_socket_with(ws, reply, done, Wording::FilesV1).await
}

async fn close_file_socket_with(
    ws: &mut ServerSocket,
    reply: Option<(Uuid, &str)>,
    done: bool,
    wording: Wording,
) {
    if let Some((id, code)) = reply {
        let _ = reject(ws, id, code, wording).await;
    }
    let frame = done.then(|| CloseFrame {
        code: CloseCode::Normal,
        reason: Utf8Bytes::from_static(""),
    });
    let _ = ws.close(frame).await;
    let _ = timeout(Duration::from_millis(250), async {
        while let Some(Ok(message)) = ws.next().await {
            if message.is_close() {
                break;
            }
        }
    })
    .await;
}

/// Deletes the part file and frees the slot on every exit, including task cancellation.
struct TransferGuard {
    core: Arc<Core>,
    token: Arc<tokio::sync::Notify>,
    part: Option<PathBuf>,
}

impl Drop for TransferGuard {
    fn drop(&mut self) {
        if let Some(part) = self.part.take() {
            let _ = std::fs::remove_file(part);
        }
        self.core.file_release(&self.token);
    }
}

/// `/v1/files`: one ticket-authorized upload (`file.begin`) or download (`file.fetch`) per socket.
async fn file_socket(core: Arc<Core>, mut ws: ServerSocket) -> Result<(), String> {
    let begin_deadline = Instant::now() + FILE_BEGIN_TIMEOUT;
    let (begin_id, accepted) = loop {
        let incoming = tokio::select! {
            _ = tokio::time::sleep_until(begin_deadline) => {
                close_file_socket(&mut ws, None, false).await;
                return Ok(());
            }
            incoming = ws.next() => incoming,
        };
        let reply = match incoming {
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            Some(Ok(Message::Text(raw))) => match parse_file_message(&raw) {
                Ok(FileClientMessage::Begin {
                    id,
                    transfer_id,
                    ticket,
                    ..
                }) => match core.file_begin(transfer_id, &ticket) {
                    Ok(accepted) => break (id, accepted),
                    Err(code) => Some((id, code)),
                },
                Ok(FileClientMessage::Fetch {
                    id,
                    transfer_id,
                    ticket,
                    ..
                }) => match core.outgoing_fetch(transfer_id, &ticket) {
                    Ok(fetched) => return send_file(core, ws, id, fetched).await,
                    Err(code) => Some((id, code)),
                },
                Ok(other) => Some((other.id(), "invalid_message")),
                Err(reply) => Some(reply),
            },
            Some(Ok(Message::Binary(_))) | Some(Err(WsError::Capacity(_))) => {
                Some((Uuid::nil(), "invalid_message"))
            }
            _ => None,
        };
        close_file_socket(&mut ws, reply, false).await;
        return Ok(());
    };

    let transfer_id = accepted.transfer_id;
    let image = accepted.clipboard.is_some();
    let wording = if image {
        Wording::Image
    } else if accepted.kind == FileKind::Any {
        Wording::FilesV2
    } else {
        Wording::FilesV1
    };
    let _awake = crate::power::awake();
    let mut guard = TransferGuard {
        core: core.clone(),
        token: accepted.abort.clone(),
        part: None,
    };
    let part = files::part_path(&accepted.folder, transfer_id);
    let target = part.clone();
    let file = match tokio::task::spawn_blocking(move || files::create_part(&target)).await {
        Ok(Ok(file)) => file,
        Ok(Err(error)) => {
            let abort = FileAbort::Storage(begin_id, error.to_string());
            core.diagnostic(&abort.diagnostic(image));
            drop(guard);
            close_file_socket(&mut ws, abort.reply(), false).await;
            return Ok(());
        }
        Err(_) => {
            drop(guard);
            close_file_socket(&mut ws, Some((begin_id, "internal")), false).await;
            return Ok(());
        }
    };
    guard.part = Some(part.clone());
    send(
        &mut ws,
        json!({"v":1,"type":"file.accept","id":begin_id,"transferId":transfer_id}),
    )
    .await?;

    let outcome = receive_file(&core, &mut ws, &accepted, file, &part).await;
    match outcome {
        Ok((end_id, Received::Image(bytes))) => {
            // Decoding and the clipboard block; the part file goes with the guard.
            let checked = core.clone();
            let accepted = Arc::new(accepted);
            let target = accepted.clone();
            let written = tokio::task::spawn_blocking(move || {
                checked.clipboard_image_received(&target, &bytes)
            })
            .await
            .unwrap_or(Err("internal"));
            drop(guard);
            match written {
                Ok(()) => {
                    let done = json!({"v":1,"type":"file.done","id":end_id,"transferId":transfer_id,"name":"Área de transferência"});
                    if send(&mut ws, done).await.is_ok() {
                        close_file_socket(&mut ws, None, true).await;
                    }
                }
                Err(code) => {
                    if code == "too_large" {
                        // The bytes fit 32 MiB; the pixel count did not.
                        let frame = protocol::wire_error(
                            end_id,
                            code,
                            "Imagem excede 50 megapixels",
                            false,
                        );
                        let _ = send(&mut ws, frame).await;
                        close_file_socket(&mut ws, None, false).await;
                    } else {
                        close_file_socket_with(&mut ws, Some((end_id, code)), false, wording).await;
                    }
                }
            }
        }
        Ok((end_id, Received::Saved(name))) => {
            guard.part = None;
            core.file_completed(&accepted.abort, &name, accepted.size);
            drop(guard);
            let done =
                json!({"v":1,"type":"file.done","id":end_id,"transferId":transfer_id,"name":name});
            if send(&mut ws, done).await.is_ok() {
                close_file_socket(&mut ws, None, true).await;
            }
        }
        Err(abort) => {
            drop(guard);
            core.diagnostic(&abort.diagnostic(image));
            close_file_socket_with(&mut ws, abort.reply(), false, wording).await;
        }
    }
    Ok(())
}

/// Streams binary frames to a blocking writer, then verifies and saves the file.
/// A clipboard.v2 image is read back into memory instead of going to Downloads.
async fn receive_file(
    core: &Core,
    ws: &mut ServerSocket,
    accepted: &Accepted,
    file: std::fs::File,
    part: &std::path::Path,
) -> Result<(Uuid, Received), FileAbort> {
    let (chunks, queue) = mpsc::channel::<Bytes>(FILE_QUEUE_CHUNKS);
    let writer = tokio::task::spawn_blocking(move || files::write_part(file, queue));
    let mut head = Vec::with_capacity(FILE_HEAD_BYTES);
    let mut received = 0u64;
    let mut reported = 0u64;
    let total_deadline =
        Instant::now() + files::total_timeout(accepted.size, accepted.kind == FileKind::Any);
    let mut idle_deadline = Instant::now() + FILE_IDLE_TIMEOUT;
    let mut check = tokio::time::interval(Duration::from_secs(1));
    let end = loop {
        tokio::select! {
            _ = accepted.abort.notified() => {
                break Err(core.file_check(accepted, received).err().map_or(FileAbort::Cancelled, |code| FileAbort::from_check(Uuid::nil(), code)));
            }
            _ = tokio::time::sleep_until(idle_deadline) => break Err(FileAbort::Timeout),
            _ = tokio::time::sleep_until(total_deadline) => break Err(FileAbort::Timeout),
            _ = check.tick() => {
                if let Err(code) = core.file_check(accepted, received) {break Err(FileAbort::from_check(Uuid::nil(), code));}
            }
            incoming = ws.next() => {
                idle_deadline = Instant::now() + FILE_IDLE_TIMEOUT;
                let message = match incoming {
                    Some(Ok(message)) => message,
                    Some(Err(WsError::Capacity(_))) => break Err(FileAbort::Reply(Uuid::nil(), "invalid_message")),
                    _ => break Err(FileAbort::Closed),
                };
                match message {
                    Message::Binary(chunk) => {
                        if chunk.is_empty() {break Err(FileAbort::Reply(Uuid::nil(), "invalid_message"));}
                        let total = received + chunk.len() as u64;
                        if total > accepted.size {break Err(FileAbort::Reply(Uuid::nil(), "too_large"));}
                        if let Err(code) = core.file_check(accepted, total) {break Err(FileAbort::from_check(Uuid::nil(), code));}
                        if head.len() < FILE_HEAD_BYTES {head.extend_from_slice(&chunk[..chunk.len().min(FILE_HEAD_BYTES - head.len())]);}
                        // A failed send means the writer stopped on an I/O error.
                        if chunks.send(chunk).await.is_err() {break Err(FileAbort::Storage(Uuid::nil(), "falha de escrita".into()));}
                        received = total;
                        if received - reported >= FILE_PROGRESS_STEP || received == accepted.size {
                            reported = received;
                            let progress = json!({"v":1,"type":"file.progress","transferId":accepted.transfer_id,"received":received});
                            if send(ws, progress).await.is_err() {break Err(FileAbort::Closed);}
                        }
                    }
                    Message::Text(raw) => match parse_file_message(&raw) {
                        Ok(FileClientMessage::End {id, transfer_id, sha256, ..}) if transfer_id == accepted.transfer_id => {
                            if !files::is_sha256_hex(&sha256) {break Err(FileAbort::Reply(id, "invalid_message"));}
                            if received != accepted.size {break Err(FileAbort::Reply(id, "file_integrity"));}
                            break Ok((id, sha256));
                        }
                        Ok(other) => break Err(FileAbort::Reply(other.id(), "invalid_message")),
                        Err((id, code)) => break Err(FileAbort::Reply(id, code)),
                    },
                    Message::Close(_) => break Err(FileAbort::Closed),
                    Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
                }
            }
        }
    };
    // Dropping the sender ends the writer; it closes the file before anyone deletes it.
    drop(chunks);
    let written = writer.await;
    let (end_id, sha256) = match (end, written.as_ref()) {
        (Err(FileAbort::Storage(id, _)), Ok(Err(error))) => {
            return Err(FileAbort::Storage(id, error.to_string()))
        }
        (end, _) => end?,
    };
    let written = match written {
        Ok(Ok(written)) => written,
        Ok(Err(error)) => return Err(FileAbort::Storage(end_id, error.to_string())),
        Err(_) => return Err(FileAbort::Reply(end_id, "internal")),
    };
    if hex::encode(written.digest) != sha256 {
        return Err(FileAbort::Reply(end_id, "file_integrity"));
    }
    if !accepted.kind.signature_matches(&head) {
        return Err(FileAbort::Reply(end_id, "file_type"));
    }
    if let Err(code) = core.file_check(accepted, received) {
        return Err(FileAbort::from_check(end_id, code));
    }
    if accepted.clipboard.is_some() {
        let part = part.to_path_buf();
        return match tokio::task::spawn_blocking(move || {
            drop(written);
            std::fs::read(&part)
        })
        .await
        {
            Ok(Ok(bytes)) => Ok((end_id, Received::Image(bytes))),
            Ok(Err(error)) => Err(FileAbort::Storage(end_id, error.to_string())),
            Err(_) => Err(FileAbort::Reply(end_id, "internal")),
        };
    }
    let (part, folder, name, transfer_id) = (
        part.to_path_buf(),
        accepted.folder.clone(),
        accepted.name.clone(),
        accepted.transfer_id,
    );
    match tokio::task::spawn_blocking(move || {
        files::finalize(written, &part, &folder, &name, transfer_id)
    })
    .await
    {
        Ok(Ok(saved)) => Ok((end_id, Received::Saved(saved))),
        Ok(Err(error)) => Err(FileAbort::Storage(end_id, error)),
        Err(_) => Err(FileAbort::Reply(end_id, "internal")),
    }
}

/// Why a PC → phone transfer stopped: the frame for the phone, if any, and the panel's reason.
struct SendAbort {
    frame: Option<Value>,
    reason: String,
}

impl SendAbort {
    fn new(frame: Option<Value>, reason: &str) -> Self {
        Self {
            frame,
            reason: reason.to_string(),
        }
    }
    fn reply(id: Uuid, code: &str, reason: &str) -> Self {
        let (message, retryable) = protocol::error_text(code, Wording::FilesV2);
        Self::new(
            Some(protocol::wire_error(id, code, message, retryable)),
            reason,
        )
    }
    fn closed() -> Self {
        Self::new(None, "O celular encerrou o recebimento.")
    }
    fn stopped(fetched: &Fetched) -> Self {
        if fetched
            .abort
            .by_user
            .load(std::sync::atomic::Ordering::Acquire)
        {
            let frame =
                protocol::wire_error(Uuid::nil(), "unavailable", "Envio cancelado no PC.", false);
            Self::new(Some(frame), "Envio cancelado.")
        } else {
            Self::new(None, "A conexão com o celular terminou.")
        }
    }
    fn from_phone(code: &str) -> Self {
        let reason = match code {
            "storage" => "Sem espaço ou falha ao gravar no celular.",
            "file_integrity" => "O arquivo chegou corrompido ao celular.",
            _ => "O celular recusou o arquivo.",
        };
        Self::new(None, reason)
    }
}

/// files.v2 `/v1/files` after `file.fetch`: streams the announced file to the phone.
async fn send_file(
    core: Arc<Core>,
    mut ws: ServerSocket,
    fetch_id: Uuid,
    fetched: Fetched,
) -> Result<(), String> {
    let _awake = crate::power::awake();
    let outcome = stream_file(&core, &mut ws, fetch_id, &fetched).await;
    let (frame, done) = match &outcome {
        Ok(_) => (None, true),
        Err(abort) => {
            // A clipboard image is often replaced by a newer copy; that is no failure.
            if !fetched.clipboard {
                core.diagnostic(&format!(
                    "{} não foi enviado ao celular: {}",
                    fetched.name, abort.reason
                ));
            }
            (abort.frame.clone(), false)
        }
    };
    core.outgoing_finish(&fetched, outcome.map_err(|abort| abort.reason));
    if let Some(frame) = frame {
        let _ = send(&mut ws, frame).await;
    }
    close_file_socket(&mut ws, None, done).await;
    Ok(())
}

async fn stream_file(
    core: &Core,
    ws: &mut ServerSocket,
    fetch_id: Uuid,
    fetched: &Fetched,
) -> Result<String, SendAbort> {
    let accept = json!({"v":1,"type":"file.accept","id":fetch_id,"transferId":fetched.transfer_id});
    send(ws, accept).await.map_err(|_| SendAbort::closed())?;
    let (chunks_tx, mut chunks) = mpsc::channel::<Bytes>(FILE_QUEUE_CHUNKS);
    let (path, size) = (fetched.path.clone(), fetched.size);
    let mut reader =
        tokio::task::spawn_blocking(move || files::read_for_send(&path, size, chunks_tx));
    let window = protocol::FILE_WINDOW_BYTES as u64;
    let chunk_bytes = protocol::FILE_CHUNK_BYTES as u64;
    let (mut sent, mut confirmed) = (0u64, 0u64);
    let mut end_id: Option<Uuid> = None;
    let total_deadline = Instant::now() + files::total_timeout(size, true);
    let mut idle_deadline = Instant::now() + FILE_IDLE_TIMEOUT;
    let mut check = tokio::time::interval(Duration::from_secs(1));
    loop {
        let room = end_id.is_none() && sent - confirmed + chunk_bytes <= window;
        tokio::select! {
            _ = fetched.abort.notify.notified() => return Err(SendAbort::stopped(fetched)),
            _ = tokio::time::sleep_until(idle_deadline) => return Err(SendAbort::reply(Uuid::nil(), "internal", "O celular parou de responder.")),
            _ = tokio::time::sleep_until(total_deadline) => return Err(SendAbort::reply(Uuid::nil(), "internal", "Tempo esgotado.")),
            _ = check.tick() => {
                if core.outgoing_check(fetched, confirmed).is_err() {return Err(SendAbort::stopped(fetched));}
            }
            chunk = chunks.recv(), if room => match chunk {
                Some(chunk) => {
                    sent += chunk.len() as u64;
                    if ws.send(Message::Binary(chunk)).await.is_err() {return Err(SendAbort::closed());}
                }
                // The reader dropped its sender: everything was read, or it failed.
                None => match (&mut reader).await {
                    Ok(Ok(digest)) if sent == size => {
                        let id = Uuid::new_v4();
                        end_id = Some(id);
                        let end = json!({"v":1,"type":"file.end","id":id,"transferId":fetched.transfer_id,"sha256":hex::encode(digest)});
                        if send(ws, end).await.is_err() {return Err(SendAbort::closed());}
                    }
                    Ok(Ok(_)) | Ok(Err(_)) => return Err(SendAbort::reply(Uuid::nil(), "storage", "O arquivo mudou ou não pôde ser lido no PC.")),
                    Err(_) => return Err(SendAbort::reply(Uuid::nil(), "internal", "Falha ao ler o arquivo.")),
                },
            },
            incoming = ws.next() => {
                let message = match incoming {
                    Some(Ok(message)) => message,
                    Some(Err(WsError::Capacity(_))) => return Err(SendAbort::reply(Uuid::nil(), "invalid_message", "O celular respondeu fora do protocolo.")),
                    _ => return Err(SendAbort::closed()),
                };
                match message {
                    Message::Text(raw) => match parse_file_message(&raw) {
                        Ok(FileClientMessage::Progress { transfer_id, received, .. }) if transfer_id == fetched.transfer_id && received >= confirmed && received <= sent => {
                            if received > confirmed {idle_deadline = Instant::now() + FILE_IDLE_TIMEOUT;}
                            confirmed = received;
                            if core.outgoing_check(fetched, confirmed).is_err() {return Err(SendAbort::stopped(fetched));}
                        }
                        Ok(FileClientMessage::Done { id, transfer_id, name, .. }) if Some(id) == end_id && transfer_id == fetched.transfer_id && confirmed == size => {
                            let valid = !name.is_empty() && name.len() <= 1024 && !name.chars().any(char::is_control);
                            return if valid {Ok(name)} else {Err(SendAbort::reply(id, "invalid_message", "O celular respondeu fora do protocolo."))};
                        }
                        Ok(FileClientMessage::Error { code, .. }) => return Err(SendAbort::from_phone(&code)),
                        Ok(other) => return Err(SendAbort::reply(other.id(), "invalid_message", "O celular respondeu fora do protocolo.")),
                        Err((id, code)) => return Err(SendAbort::reply(id, code, "O celular respondeu fora do protocolo.")),
                    },
                    Message::Binary(_) => return Err(SendAbort::reply(Uuid::nil(), "invalid_message", "O celular respondeu fora do protocolo.")),
                    Message::Close(_) => return Err(SendAbort::closed()),
                    Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::secrets::Secrets;
    use rustls::{pki_types::ServerName, ClientConfig, RootCertStore};
    use tokio_rustls::TlsConnector;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    type TestSocket =
        tokio_tungstenite::WebSocketStream<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>;

    async fn connect(port: u16, cert: &str) -> TestSocket {
        connect_features(port, cert, false).await
    }

    async fn connect_features(port: u16, cert: &str, input: bool) -> TestSocket {
        let headers: &[(&'static str, &str)] = if input {
            &[("X-PC-Remote-Features", "input.v1")]
        } else {
            &[]
        };
        connect_with(port, cert, "/v1/ws", headers).await.unwrap()
    }

    /// Repeated header names become separate header lines, as OkHttp `addHeader` sends them.
    async fn connect_with(
        port: u16,
        cert: &str,
        path: &str,
        headers: &[(&'static str, &str)],
    ) -> Result<TestSocket, WsError> {
        install_crypto_provider();
        let mut roots = RootCertStore::empty();
        for cert in rustls_pemfile::certs(&mut Cursor::new(cert)) {
            roots.add(cert.unwrap()).unwrap();
        }
        let config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        let tls = TlsConnector::from(Arc::new(config))
            .connect(ServerName::try_from("pc-remote.local").unwrap(), stream)
            .await
            .unwrap();
        let mut request = format!("wss://pc-remote.local{path}")
            .into_client_request()
            .unwrap();
        for (name, value) in headers {
            request.headers_mut().append(*name, value.parse().unwrap());
        }
        tokio_tungstenite::client_async(request, tls)
            .await
            .map(|(ws, _)| ws)
    }

    /// The next text message, without skipping any: used to assert ordering.
    async fn next_text(ws: &mut TestSocket) -> Value {
        timeout(Duration::from_secs(5), async {
            while let Some(message) = ws.next().await {
                match message.unwrap() {
                    Message::Text(text) => return serde_json::from_str(&text).unwrap(),
                    Message::Ping(_) | Message::Pong(_) => {}
                    other => panic!("Unexpected message {other:?}"),
                }
            }
            panic!("Socket closed before expected response");
        })
        .await
        .expect("Expected response timeout")
    }

    /// Waits for the server close frame and returns its code, if any.
    async fn closed(ws: &mut TestSocket) -> Option<CloseCode> {
        timeout(Duration::from_secs(5), async {
            while let Some(message) = ws.next().await {
                match message {
                    Ok(Message::Close(frame)) => return frame.map(|frame| frame.code),
                    Ok(Message::Text(text)) => panic!("Unexpected text {text}"),
                    Ok(_) => {}
                    Err(_) => return None,
                }
            }
            None
        })
        .await
        .expect("Expected close timeout")
    }

    struct TestServer {
        core: Arc<Core>,
        port: u16,
        certificate_pem: String,
        server: tokio::task::JoinHandle<()>,
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.server.abort();
            if let Some(root) = &self.core.files_root {
                let _ = std::fs::remove_dir_all(root);
            }
        }
    }

    /// Ephemeral server with one paired device; files land in a temporary folder.
    async fn file_server() -> (TestServer, Uuid, String) {
        let cert = rcgen::generate_simple_self_signed(vec!["pc-remote.local".into()]).unwrap();
        let certificate_pem = cert.cert.pem();
        let secrets = Secrets {
            pc_id: Uuid::new_v4(),
            certificate_pem: certificate_pem.clone(),
            private_key_pem: cert.signing_key.serialize_pem(),
            devices: Vec::new(),
        };
        let (config, fingerprint) =
            tls_config(&secrets.certificate_pem, &secrets.private_key_pem).unwrap();
        let core = Core::ephemeral(secrets, fingerprint);
        let device = Uuid::new_v4();
        let token = crate::core::random_hex();
        {
            let mut s = core.state.lock().unwrap();
            s.secrets.devices.push(crate::secrets::Device {
                id: device,
                name: "Files test".into(),
                token_hash: crate::secrets::token_hash(&token),
            });
        }
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let server_core = core.clone();
        let server = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                let core = server_core.clone();
                tokio::spawn(async move {
                    if let Ok(tls) = acceptor.accept(stream).await {
                        let _ = handle(core, tls).await;
                    }
                });
            }
        });
        let server = TestServer {
            core,
            port,
            certificate_pem,
            server,
        };
        (server, device, token)
    }

    const BOTH_FEATURES: &[(&'static str, &str)] = &[
        ("X-PC-Remote-Features", "input.v1"),
        ("X-PC-Remote-Features", "files.v1"),
    ];

    async fn files_session(server: &TestServer, device: Uuid, token: &str) -> TestSocket {
        let mut control = connect_with(
            server.port,
            &server.certificate_pem,
            "/v1/ws",
            BOTH_FEATURES,
        )
        .await
        .unwrap();
        emit(
            &mut control,
            json!({"v":1,"type":"hello","id":Uuid::new_v4(),"deviceId":device,"token":token}),
        )
        .await;
        receive(&mut control, "welcome").await;
        receive(&mut control, "state").await;
        control
    }

    /// Sends `file.offer`; returns the ticket after checking that `file.ticket` precedes `ack`.
    async fn offer(
        control: &mut TestSocket,
        transfer: Uuid,
        name: &str,
        size: usize,
        mime: &str,
    ) -> String {
        let id = Uuid::new_v4();
        emit(control, json!({"v":1,"type":"request","id":id,"action":{"kind":"file.offer","transferId":transfer,"name":name,"size":size,"mime":mime}})).await;
        let ticket = next_text(control).await;
        assert_eq!(ticket["type"], "file.ticket", "{ticket}");
        assert_eq!(ticket["id"], id.to_string());
        assert_eq!(ticket["transferId"], transfer.to_string());
        assert_eq!(ticket["chunkBytes"], 65_536);
        assert_eq!(ticket["windowBytes"], 1_048_576);
        let ack = next_text(control).await;
        assert_eq!(ack, json!({"v":1,"type":"ack","id":id}));
        let ticket = ticket["ticket"].as_str().unwrap().to_owned();
        assert!(files::is_sha256_hex(&ticket));
        ticket
    }

    async fn begin(server: &TestServer, transfer: Uuid, ticket: &str) -> TestSocket {
        let mut upload = connect_with(server.port, &server.certificate_pem, "/v1/files", &[])
            .await
            .unwrap();
        let id = Uuid::new_v4();
        emit(
            &mut upload,
            json!({"v":1,"type":"file.begin","id":id,"transferId":transfer,"ticket":ticket}),
        )
        .await;
        assert_eq!(
            next_text(&mut upload).await,
            json!({"v":1,"type":"file.accept","id":id,"transferId":transfer})
        );
        upload
    }

    fn part_files(folder: &std::path::Path) -> Vec<String> {
        std::fs::read_dir(folder)
            .unwrap()
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| name.ends_with(".pcremote-part"))
            .collect()
    }

    /// Progress is only reported every 256 KiB, so partial uploads are observed via the panel state.
    async fn received(core: &Core, bytes: u64) {
        wait_until(|| {
            core.status()
                .incoming_file
                .is_some_and(|file| file.received == bytes)
        })
        .await;
    }

    async fn wait_until(mut condition: impl FnMut() -> bool) {
        timeout(Duration::from_secs(5), async {
            while !condition() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("condition timeout");
    }

    const PHONE_06: &[(&'static str, &str)] = &[
        ("X-PC-Remote-Features", "input.v1"),
        ("X-PC-Remote-Features", "files.v1"),
        ("X-PC-Remote-Features", "files.v2"),
        ("X-PC-Remote-Features", "search.v1"),
        ("X-PC-Remote-Features", "info.v1"),
    ];

    async fn session_06(
        server: &TestServer,
        device: Uuid,
        token: &str,
    ) -> (TestSocket, Value, Value) {
        let mut control = connect_with(server.port, &server.certificate_pem, "/v1/ws", PHONE_06)
            .await
            .unwrap();
        emit(
            &mut control,
            json!({"v":1,"type":"hello","id":Uuid::new_v4(),"deviceId":device,"token":token}),
        )
        .await;
        let welcome = receive(&mut control, "welcome").await;
        let state = receive(&mut control, "state").await;
        (control, welcome, state)
    }

    /// Deterministic, not compressible by accident, and different per offset.
    fn pattern(size: usize) -> Vec<u8> {
        (0..size)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
            .collect()
    }

    #[tokio::test]
    async fn real_tls_v2_welcome_and_any_type_upload_above_the_old_limit() {
        let (server, device, token) = file_server().await;
        let core = server.core.clone();
        // The app polls every 650 ms, which keeps the resume detector from mistaking a
        // long upload on a busy test machine for a suspended PC.
        let poll = core.clone();
        let ticker = tokio::spawn(async move {
            loop {
                poll.check_resume();
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        });
        let (mut control, welcome, state) = session_06(&server, device, &token).await;
        assert_eq!(
            welcome["capabilities"],
            json!([
                "audio",
                "media",
                "clipboard.text",
                "input.control",
                "file.transfer",
                "file.receive",
                "search.youtube"
            ])
        );
        assert_eq!(welcome["os"], crate::system::os_name());
        assert_eq!(
            state["files"],
            json!({"allowed":true,"maxBytes":1_073_741_824})
        );

        // 60 MB of a type files.v1 refused, sent inside the 1 MiB window.
        let payload = pattern(60 * 1024 * 1024 + 123);
        let transfer = Uuid::new_v4();
        let ticket = offer(
            &mut control,
            transfer,
            "Viagem 2026.mkv",
            payload.len(),
            "video/x-matroska",
        )
        .await;
        let mut upload = begin(&server, transfer, &ticket).await;
        let mut sent = 0usize;
        let mut confirmed = 0usize;
        for chunk in payload.chunks(protocol::FILE_CHUNK_BYTES) {
            while sent + chunk.len() - confirmed > protocol::FILE_WINDOW_BYTES {
                let progress = next_text(&mut upload).await;
                assert_eq!(progress["type"], "file.progress");
                confirmed = progress["received"].as_u64().unwrap() as usize;
            }
            upload
                .send(Message::Binary(chunk.to_vec().into()))
                .await
                .unwrap();
            sent += chunk.len();
        }
        let digest = hex::encode(Sha256::digest(&payload));
        let end = Uuid::new_v4();
        emit(
            &mut upload,
            json!({"v":1,"type":"file.end","id":end,"transferId":transfer,"sha256":digest}),
        )
        .await;
        let done = loop {
            let message = next_text(&mut upload).await;
            if message["type"] != "file.progress" {
                break message;
            }
        };
        assert_eq!(
            done,
            json!({"v":1,"type":"file.done","id":end,"transferId":transfer,"name":"Viagem 2026.mkv"})
        );
        let folder = core.received_folder().unwrap();
        let saved = std::fs::read(folder.join("Viagem 2026.mkv")).unwrap();
        assert_eq!(hex::encode(Sha256::digest(&saved)), digest);
        if let Ok(motw) = std::fs::read(folder.join("Viagem 2026.mkv:Zone.Identifier")) {
            assert!(motw.starts_with(b"[ZoneTransfer]"));
        }

        // Shell-active files are refused and the 1 GiB limit applies with its own wording.
        let id = Uuid::new_v4();
        emit(&mut control, json!({"v":1,"type":"request","id":id,"action":{"kind":"file.offer","transferId":Uuid::new_v4(),"name":"atalho.lnk","size":10,"mime":"application/octet-stream"}})).await;
        assert_eq!(receive(&mut control, "error").await["code"], "file_type");
        let id = Uuid::new_v4();
        emit(&mut control, json!({"v":1,"type":"request","id":id,"action":{"kind":"file.offer","transferId":Uuid::new_v4(),"name":"a.mkv","size":1_073_741_825u64,"mime":"video/x-matroska"}})).await;
        let error = receive(&mut control, "error").await;
        assert_eq!(
            (error["code"].as_str(), error["message"].as_str()),
            (Some("too_large"), Some("Arquivo excede 1 GB"))
        );
        ticker.abort();
    }

    #[tokio::test]
    async fn real_tls_push_to_phone_respects_the_window_verifies_and_can_be_cancelled() {
        let (server, device, token) = file_server().await;
        let core = server.core.clone();
        let folder = files::tests::temp_folder("push-source");
        let payload = pattern(3 * 1024 * 1024 + 77);
        let source = folder.join("Relatório final.pdf");
        std::fs::write(&source, &payload).unwrap();
        // Queued before any phone is connected: it waits.
        assert!(core.enqueue_outgoing(vec![source.clone()]).is_empty());
        assert!(core.status().outgoing.is_none());
        let (mut control, ..) = session_06(&server, device, &token).await;
        let push = receive(&mut control, "file.push").await;
        assert_eq!(push["name"], "Relatório final.pdf");
        assert_eq!(push["mime"], "application/pdf");
        assert_eq!(push["size"], payload.len());
        let transfer = push["transferId"].as_str().unwrap().to_owned();
        let ticket = push["ticket"].as_str().unwrap().to_owned();

        let mut download = connect_with(server.port, &server.certificate_pem, "/v1/files", &[])
            .await
            .unwrap();
        let fetch = Uuid::new_v4();
        emit(
            &mut download,
            json!({"v":1,"type":"file.fetch","id":fetch,"transferId":transfer,"ticket":ticket}),
        )
        .await;
        assert_eq!(
            next_text(&mut download).await,
            json!({"v":1,"type":"file.accept","id":fetch,"transferId":transfer})
        );
        let mut received = Vec::new();
        let mut reported = 0usize;
        let end = loop {
            let message = timeout(Duration::from_secs(5), download.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            match message {
                Message::Binary(chunk) => {
                    assert!(!chunk.is_empty() && chunk.len() <= protocol::FILE_CHUNK_BYTES);
                    received.extend_from_slice(&chunk);
                    // The PC never runs more than one window ahead of what the phone confirmed.
                    assert!(received.len() - reported <= protocol::FILE_WINDOW_BYTES);
                    if received.len() - reported >= 256 * 1024 || received.len() == payload.len() {
                        reported = received.len();
                        emit(&mut download, json!({"v":1,"type":"file.progress","transferId":transfer,"received":reported})).await;
                    }
                }
                Message::Text(text) => break serde_json::from_str::<Value>(&text).unwrap(),
                other => panic!("unexpected {other:?}"),
            }
        };
        assert_eq!(end["type"], "file.end");
        assert_eq!(received, payload);
        assert_eq!(end["sha256"], hex::encode(Sha256::digest(&payload)));
        wait_until(|| {
            core.status()
                .outgoing
                .is_some_and(|file| file.sent == payload.len() as u64)
        })
        .await;
        emit(&mut download, json!({"v":1,"type":"file.done","id":end["id"],"transferId":transfer,"name":"Relatório final (1).pdf"})).await;
        assert_eq!(closed(&mut download).await, Some(CloseCode::Normal));
        wait_until(|| {
            core.status()
                .outgoing_result
                .is_some_and(|result| result.ok)
        })
        .await;
        let status = core.status();
        assert_eq!(
            status.file_history[0].direction,
            crate::core::TransferDirection::ToPhone
        );
        assert_eq!(status.file_history[0].name, "Relatório final.pdf");

        // Cancelled on the PC while sending: the phone hears it and the next file follows.
        let second = folder.join("segundo.bin");
        std::fs::write(&second, pattern(2 * 1024 * 1024)).unwrap();
        core.enqueue_outgoing(vec![source.clone(), second]);
        let push = receive(&mut control, "file.push").await;
        let transfer = push["transferId"].as_str().unwrap().to_owned();
        let mut download = connect_with(server.port, &server.certificate_pem, "/v1/files", &[])
            .await
            .unwrap();
        emit(&mut download, json!({"v":1,"type":"file.fetch","id":Uuid::new_v4(),"transferId":transfer,"ticket":push["ticket"]})).await;
        assert_eq!(next_text(&mut download).await["type"], "file.accept");
        wait_until(|| core.status().outgoing.is_some_and(|file| !file.waiting)).await;
        core.cancel_outgoing();
        let error = loop {
            match timeout(Duration::from_secs(5), download.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap()
            {
                Message::Text(text) => break serde_json::from_str::<Value>(&text).unwrap(),
                Message::Binary(_) => {}
                other => panic!("unexpected {other:?}"),
            }
        };
        assert_eq!(
            (error["code"].as_str(), error["message"].as_str()),
            (Some("unavailable"), Some("Envio cancelado no PC."))
        );
        let next = receive(&mut control, "file.push").await;
        assert_eq!(next["name"], "segundo.bin");
        // The phone declines it: no space.
        let id = Uuid::new_v4();
        emit(&mut control, json!({"v":1,"type":"request","id":id,"action":{"kind":"file.decline","transferId":next["transferId"],"code":"storage"}})).await;
        assert_eq!(receive(&mut control, "ack").await["id"], id.to_string());
        let result = core.status().outgoing_result.unwrap();
        assert!(
            !result.ok && result.message.contains("Sem espaço"),
            "{}",
            result.message
        );
        // A used ticket cannot be fetched again.
        let mut replay = connect_with(server.port, &server.certificate_pem, "/v1/files", &[])
            .await
            .unwrap();
        emit(&mut replay, json!({"v":1,"type":"file.fetch","id":Uuid::new_v4(),"transferId":transfer,"ticket":push["ticket"]})).await;
        assert_eq!(next_text(&mut replay).await["code"], "unauthorized");
        std::fs::remove_dir_all(folder).unwrap();
    }

    const PHONE_061: &[(&'static str, &str)] = &[
        ("X-PC-Remote-Features", "input.v1"),
        ("X-PC-Remote-Features", "files.v1"),
        ("X-PC-Remote-Features", "files.v2"),
        ("X-PC-Remote-Features", "search.v1"),
        ("X-PC-Remote-Features", "info.v1"),
        ("X-PC-Remote-Features", "power.v1"),
        ("X-PC-Remote-Features", "clipboard.v2"),
    ];

    async fn session_061(
        server: &TestServer,
        device: Uuid,
        token: &str,
    ) -> (TestSocket, Value, Value) {
        let mut control = connect_with(server.port, &server.certificate_pem, "/v1/ws", PHONE_061)
            .await
            .unwrap();
        emit(
            &mut control,
            json!({"v":1,"type":"hello","id":Uuid::new_v4(),"deviceId":device,"token":token}),
        )
        .await;
        let welcome = receive(&mut control, "welcome").await;
        let state = receive(&mut control, "state").await;
        (control, welcome, state)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn real_tls_power_is_negotiated_acked_before_its_snapshot_and_blocked_by_interromper() {
        let (server, device, token) = file_server().await;
        let core = server.core.clone();
        let ran = Arc::new(std::sync::Mutex::new(Vec::new()));
        core.set_power_system(Box::new(crate::energy::tests::Recorder {
            ran: ran.clone(),
            sleeps: true,
            fails: false,
        }));
        // A 0.6.0 phone: no capability, no field, and power actions are unavailable.
        let (mut old, welcome, state) = session_06(&server, device, &token).await;
        assert!(!welcome["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("power.control")));
        assert!(state.get("power").is_none());
        let id = Uuid::new_v4();
        emit(&mut old, json!({"v":1,"type":"request","id":id,"action":{"kind":"power.run","action":"lock","delaySeconds":0}})).await;
        assert_eq!(
            next_text(&mut old).await,
            json!({"v":1,"type":"error","id":id,"code":"unavailable","message":"Ação indisponível neste dispositivo ou player","retryable":false})
        );
        drop(old);

        let (mut control, welcome, state) = session_061(&server, device, &token).await;
        assert_eq!(
            welcome["capabilities"],
            json!([
                "audio",
                "media",
                "clipboard.text",
                "input.control",
                "file.transfer",
                "file.receive",
                "search.youtube",
                "power.control",
                "clipboard.image"
            ])
        );
        assert_eq!(state["power"], json!({"allowed":true,"scheduled":null}));
        // Scheduled: the ack, then the snapshot with the schedule.
        let id = Uuid::new_v4();
        emit(&mut control, json!({"v":1,"type":"request","id":id,"action":{"kind":"power.run","action":"shutdown","delaySeconds":1800}})).await;
        assert_eq!(
            next_text(&mut control).await,
            json!({"v":1,"type":"ack","id":id})
        );
        let state = next_text(&mut control).await;
        assert_eq!(state["type"], "state");
        assert_eq!(state["power"]["scheduled"]["action"], "shutdown");
        let remaining = state["power"]["scheduled"]["remainingSeconds"]
            .as_u64()
            .unwrap();
        assert!((1799..=1800).contains(&remaining), "{remaining}");
        assert!(core.status().power_scheduled.is_some());
        // Field rules.
        let id = Uuid::new_v4();
        emit(&mut control, json!({"v":1,"type":"request","id":id,"action":{"kind":"power.run","action":"lock","delaySeconds":60}})).await;
        assert_eq!(next_text(&mut control).await["code"], "invalid_message");
        // Cancel: ack, then the snapshot without it; a second cancel is only an ack.
        let id = Uuid::new_v4();
        emit(
            &mut control,
            json!({"v":1,"type":"request","id":id,"action":{"kind":"power.cancel"}}),
        )
        .await;
        assert_eq!(
            next_text(&mut control).await,
            json!({"v":1,"type":"ack","id":id})
        );
        assert_eq!(
            next_text(&mut control).await["power"],
            json!({"allowed":true,"scheduled":null})
        );
        let id = Uuid::new_v4();
        emit(
            &mut control,
            json!({"v":1,"type":"request","id":id,"action":{"kind":"power.cancel"}}),
        )
        .await;
        assert_eq!(
            next_text(&mut control).await,
            json!({"v":1,"type":"ack","id":id})
        );
        // Now: ack first, the action half a second later.
        let id = Uuid::new_v4();
        emit(&mut control, json!({"v":1,"type":"request","id":id,"action":{"kind":"power.run","action":"lock","delaySeconds":0}})).await;
        assert_eq!(
            next_text(&mut control).await,
            json!({"v":1,"type":"ack","id":id})
        );
        assert!(ran.lock().unwrap().is_empty());
        wait_until(|| !ran.lock().unwrap().is_empty()).await;
        assert_eq!(
            *ran.lock().unwrap(),
            vec![crate::protocol::PowerAction::Lock]
        );
        // "Interromper": allowed false, and power.run is refused.
        core.stop_input_with_save(|_| Ok(())).unwrap();
        assert_eq!(
            receive(&mut control, "state").await["power"],
            json!({"allowed":false,"scheduled":null})
        );
        let id = Uuid::new_v4();
        emit(&mut control, json!({"v":1,"type":"request","id":id,"action":{"kind":"power.run","action":"sleep","delaySeconds":600}})).await;
        assert_eq!(next_text(&mut control).await["code"], "unavailable");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn real_tls_clipboard_image_push_reaches_the_phone_with_its_field() {
        let (server, device, token) = file_server().await;
        let core = server.core.clone();
        let (mut control, ..) = session_061(&server, device, &token).await;
        emit(&mut control, json!({"v":1,"type":"request","id":Uuid::new_v4(),"action":{"kind":"sync.set","upload":true,"download":true,"paused":false}})).await;
        receive(&mut control, "ack").await;
        let png = crate::wic::tests::sample_png();
        let (epoch, revision) = {
            let s = core.state.lock().unwrap();
            (s.connection_epoch, s.clipboard_revision)
        };
        core.push_clipboard_image(png.clone(), device, epoch, revision, true, None)
            .unwrap();
        let push = receive(&mut control, "file.push").await;
        assert_eq!(push["name"], "Imagem do PC.png");
        assert_eq!(push["mime"], "image/png");
        assert_eq!(
            push["clipboard"],
            json!({"revision":revision,"automatic":true})
        );
        let transfer = push["transferId"].as_str().unwrap().to_owned();
        let mut download = connect_with(server.port, &server.certificate_pem, "/v1/files", &[])
            .await
            .unwrap();
        emit(&mut download, json!({"v":1,"type":"file.fetch","id":Uuid::new_v4(),"transferId":transfer,"ticket":push["ticket"]})).await;
        assert_eq!(next_text(&mut download).await["type"], "file.accept");
        let mut received = Vec::new();
        let end = loop {
            match timeout(Duration::from_secs(5), download.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap()
            {
                Message::Binary(chunk) => received.extend_from_slice(&chunk),
                Message::Text(text) => break serde_json::from_str::<Value>(&text).unwrap(),
                other => panic!("unexpected {other:?}"),
            }
        };
        assert_eq!(received, png);
        emit(
            &mut download,
            json!({"v":1,"type":"file.progress","transferId":transfer,"received":png.len()}),
        )
        .await;
        emit(&mut download, json!({"v":1,"type":"file.done","id":end["id"],"transferId":transfer,"name":"Imagem do PC.png"})).await;
        assert_eq!(closed(&mut download).await, Some(CloseCode::Normal));
        wait_until(|| {
            core.status()
                .transfers
                .first()
                .is_some_and(|entry| entry.kind == crate::core::TransferKind::Image)
        })
        .await;
        let status = core.status();
        assert!(status.file_history.is_empty());
        assert!(status.outgoing_result.is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn real_tls_clipboard_image_upload_goes_to_the_clipboard_not_downloads() {
        let (server, device, token) = file_server().await;
        let core = server.core.clone();
        let written = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = written.clone();
        core.set_image_writer(Box::new(move |png: &[u8], dib: &[u8], expected| {
            let mut all = sink.lock().unwrap();
            if expected == Some(4242) {
                return Err("conflict".into());
            }
            all.push((png.to_vec(), dib.len()));
            Ok(9000 + all.len() as u32)
        }));
        let (mut control, ..) = session_061(&server, device, &token).await;
        let png = crate::wic::tests::sample_png();
        // A manual image (Enviar o que copiei): the image limits word the errors.
        let id = Uuid::new_v4();
        emit(&mut control, json!({"v":1,"type":"request","id":id,"action":{"kind":"file.offer","transferId":Uuid::new_v4(),"name":"Imagem.png","size":33_554_433u64,"mime":"image/png","clipboard":{"eventId":Uuid::new_v4(),"automatic":false,"baseRevision":0}}})).await;
        assert_eq!(
            next_text(&mut control).await,
            json!({"v":1,"type":"error","id":id,"code":"too_large","message":"Imagem excede 32 MB","retryable":false})
        );
        let transfer = Uuid::new_v4();
        let id = Uuid::new_v4();
        emit(&mut control, json!({"v":1,"type":"request","id":id,"action":{"kind":"file.offer","transferId":transfer,"name":"Imagem.png","size":png.len(),"mime":"image/png","clipboard":{"eventId":Uuid::new_v4(),"automatic":false,"baseRevision":0}}})).await;
        let ticket = next_text(&mut control).await;
        assert_eq!(ticket["type"], "file.ticket");
        assert_eq!(
            next_text(&mut control).await,
            json!({"v":1,"type":"ack","id":id})
        );
        let mut upload = begin(&server, transfer, ticket["ticket"].as_str().unwrap()).await;
        upload
            .send(Message::Binary(png.clone().into()))
            .await
            .unwrap();
        assert_eq!(next_text(&mut upload).await["received"], png.len());
        let end = Uuid::new_v4();
        emit(&mut upload, json!({"v":1,"type":"file.end","id":end,"transferId":transfer,"sha256":hex::encode(Sha256::digest(&png))})).await;
        assert_eq!(
            next_text(&mut upload).await,
            json!({"v":1,"type":"file.done","id":end,"transferId":transfer,"name":"Área de transferência"})
        );
        assert_eq!(closed(&mut upload).await, Some(CloseCode::Normal));
        assert_eq!(written.lock().unwrap()[0].0, png);
        let state = receive(&mut control, "state").await;
        assert_eq!(state["clipboardRevision"], 1);
        let status = core.status();
        assert!(status.file_history.is_empty());
        assert_eq!(status.transfers[0].kind, crate::core::TransferKind::Image);
        // Nothing went to Downloads.
        let folder = core.received_folder().unwrap();
        assert!(std::fs::read_dir(&folder).unwrap().next().is_none());
        assert_eq!(
            std::fs::read_dir(core.clipboard_folder().unwrap())
                .unwrap()
                .count(),
            0
        );

        // An automatic image whose clipboard changed meanwhile: conflict on the file socket.
        emit(&mut control, json!({"v":1,"type":"request","id":Uuid::new_v4(),"action":{"kind":"sync.set","upload":true,"download":true,"paused":false}})).await;
        receive(&mut control, "ack").await;
        core.state.lock().unwrap().clipboard_sequence = 4242;
        let transfer = Uuid::new_v4();
        let id = Uuid::new_v4();
        emit(&mut control, json!({"v":1,"type":"request","id":id,"action":{"kind":"file.offer","transferId":transfer,"name":"Imagem.png","size":png.len(),"mime":"image/png","clipboard":{"eventId":Uuid::new_v4(),"automatic":true,"baseRevision":1}}})).await;
        let ticket = receive(&mut control, "file.ticket").await;
        let mut upload = begin(&server, transfer, ticket["ticket"].as_str().unwrap()).await;
        upload
            .send(Message::Binary(png.clone().into()))
            .await
            .unwrap();
        next_text(&mut upload).await;
        let end = Uuid::new_v4();
        emit(&mut upload, json!({"v":1,"type":"file.end","id":end,"transferId":transfer,"sha256":hex::encode(Sha256::digest(&png))})).await;
        assert_eq!(
            next_text(&mut upload).await,
            json!({"v":1,"type":"error","id":end,"code":"conflict","message":"Houve outra cópia antes desta","retryable":false})
        );
        assert_eq!(written.lock().unwrap().len(), 1);
        wait_until(|| {
            std::fs::read_dir(core.clipboard_folder().unwrap())
                .unwrap()
                .count()
                == 0
        })
        .await;
    }

    #[test]
    fn feature_header_tokens_from_every_line() {
        use tokio_tungstenite::tungstenite::http::HeaderValue;
        let parse = |values: &[&str]| {
            let values: Vec<HeaderValue> = values.iter().map(|v| v.parse().unwrap()).collect();
            Features::parse(values.iter().map(|v| v.as_bytes()))
        };
        assert_eq!(parse(&[]), Features::default());
        assert_eq!(parse(&["input.v1"]), Features::legacy(true, false));
        assert_eq!(
            parse(&["input.v1", "files.v1"]),
            Features::legacy(true, true)
        );
        assert_eq!(parse(&["input.v1, files.v1"]), Features::legacy(true, true));
        assert_eq!(parse(&["files.v1,input.v1"]), Features::legacy(true, true));
        assert_eq!(parse(&["  files.v1\t"]), Features::legacy(false, true));
        assert_eq!(
            parse(&["files.v3", "input.v1x", "INPUT.V1"]),
            Features::default()
        );
        // 0.6 phones: one line per token. files.v2 implies files.v1.
        assert_eq!(
            parse(&["input.v1", "files.v1", "files.v2", "search.v1", "info.v1"]),
            Features {
                input: true,
                files: true,
                files_v2: true,
                search: true,
                info: true,
                ..Features::default()
            }
        );
        assert_eq!(
            parse(&["files.v2"]),
            Features {
                files: true,
                files_v2: true,
                ..Features::default()
            }
        );
        assert_eq!(parse(&["files.v1;q=1"]), Features::default());
        // 0.6.1: clipboard.v2 counts only with files.v2, its transport.
        let full = parse(&["files.v2", "power.v1", "clipboard.v2"]);
        assert!(full.power && full.clipboard_image);
        assert!(!parse(&["files.v1", "clipboard.v2"]).clipboard_image);
        let raw = [b"files.v1 \xff".as_slice()];
        assert_eq!(Features::parse(raw), Features::default());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn real_tls_file_transfer_saves_marks_and_rejects_reuse() {
        let (server, device, token) = file_server().await;
        let core = server.core.clone();
        let shown = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = shown.clone();
        core.set_notifier(Box::new(move |title, body| {
            sink.lock()
                .unwrap()
                .push((title.to_string(), body.to_string()));
        }));
        let folder = core.received_folder().unwrap();
        assert!(folder.starts_with(core.files_root.as_ref().unwrap()));
        assert!(folder.ends_with("PC Remote"));
        std::fs::write(folder.join("IMG 0001.png"), b"existing").unwrap();
        let stale = files::part_path(&folder, Uuid::new_v4());
        std::fs::write(&stale, b"left over").unwrap();

        // Origin is refused on the file path too, and unknown paths get 403.
        let origin = [("Origin", "https://example.org")];
        assert!(
            connect_with(server.port, &server.certificate_pem, "/v1/files", &origin)
                .await
                .is_err()
        );
        assert!(
            connect_with(server.port, &server.certificate_pem, "/v1/upload", &[])
                .await
                .is_err()
        );

        let mut control = connect_with(
            server.port,
            &server.certificate_pem,
            "/v1/ws",
            BOTH_FEATURES,
        )
        .await
        .unwrap();
        emit(
            &mut control,
            json!({"v":1,"type":"hello","id":Uuid::new_v4(),"deviceId":device,"token":token}),
        )
        .await;
        let welcome = receive(&mut control, "welcome").await;
        assert_eq!(
            welcome["capabilities"],
            json!([
                "audio",
                "media",
                "clipboard.text",
                "input.control",
                "file.transfer"
            ])
        );
        let state = receive(&mut control, "state").await;
        assert_eq!(
            state["files"],
            json!({"allowed":true,"maxBytes":52_428_800})
        );

        let mut payload = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        payload.extend((0..300_000u32).map(|i| (i.wrapping_mul(31) % 251) as u8));
        let transfer = Uuid::new_v4();
        let ticket = offer(
            &mut control,
            transfer,
            "DCIM/IMG  0001.PNG",
            payload.len(),
            "image/png",
        )
        .await;
        assert!(
            !stale.exists(),
            "stale part files are removed before the first transfer"
        );

        let mut upload = begin(&server, transfer, &ticket).await;
        assert_eq!(core.status().incoming_file.unwrap().name, "IMG 0001.png");
        for chunk in payload.chunks(protocol::FILE_CHUNK_BYTES) {
            upload
                .send(Message::Binary(chunk.to_vec().into()))
                .await
                .unwrap();
        }
        let end_id = Uuid::new_v4();
        let sha256 = hex::encode(Sha256::digest(&payload));
        emit(
            &mut upload,
            json!({"v":1,"type":"file.end","id":end_id,"transferId":transfer,"sha256":sha256}),
        )
        .await;
        let mut progress = Vec::new();
        let done = loop {
            let message = next_text(&mut upload).await;
            match message["type"].as_str() {
                Some("file.progress") => {
                    assert_eq!(message["transferId"], transfer.to_string());
                    progress.push(message["received"].as_u64().unwrap());
                }
                Some("file.done") => break message,
                _ => panic!("Unexpected {message}"),
            }
        };
        assert_eq!(progress, vec![262_144, payload.len() as u64]);
        assert_eq!(
            done,
            json!({"v":1,"type":"file.done","id":end_id,"transferId":transfer,"name":"IMG 0001 (1).png"})
        );
        assert_eq!(closed(&mut upload).await, Some(CloseCode::Normal));

        assert_eq!(
            std::fs::read(folder.join("IMG 0001 (1).png")).unwrap(),
            payload
        );
        assert_eq!(
            std::fs::read(folder.join("IMG 0001.png")).unwrap(),
            b"existing"
        );
        if let Ok(motw) = std::fs::read(folder.join("IMG 0001 (1).png:Zone.Identifier")) {
            assert_eq!(motw, b"[ZoneTransfer]\r\nZoneId=3\r\n");
        }
        assert!(part_files(&folder).is_empty());
        let status = core.status();
        assert!(status.incoming_file.is_none());
        assert_eq!(status.file_history.len(), 1);
        assert_eq!(status.file_history[0].name, "IMG 0001 (1).png");
        assert_eq!(status.file_history[0].bytes, payload.len() as u64);
        assert_eq!(
            *shown.lock().unwrap(),
            vec![(
                "Arquivo recebido do celular".to_string(),
                "IMG 0001 (1).png foi salvo em Downloads\\PC Remote.".to_string()
            )]
        );

        // The ticket was consumed by the first file.begin.
        let mut reuse = connect_with(server.port, &server.certificate_pem, "/v1/files", &[])
            .await
            .unwrap();
        let reuse_id = Uuid::new_v4();
        emit(
            &mut reuse,
            json!({"v":1,"type":"file.begin","id":reuse_id,"transferId":transfer,"ticket":ticket}),
        )
        .await;
        let refused = next_text(&mut reuse).await;
        assert_eq!(refused["code"], "unauthorized");
        assert_eq!(refused["id"], reuse_id.to_string());
        assert_eq!(refused["retryable"], false);

        // Control-socket rejections use the file wording and send no snapshot.
        let id = Uuid::new_v4();
        emit(&mut control, json!({"v":1,"type":"request","id":id,"action":{"kind":"file.offer","transferId":Uuid::new_v4(),"name":"grande.pdf","size":52_428_801u64,"mime":"application/pdf"}})).await;
        assert_eq!(
            next_text(&mut control).await,
            json!({"v":1,"type":"error","id":id,"code":"too_large","message":"Arquivo excede 50 MB","retryable":false})
        );
        let id = Uuid::new_v4();
        emit(&mut control, json!({"v":1,"type":"request","id":id,"action":{"kind":"file.offer","transferId":Uuid::new_v4(),"name":"foto.png","size":10,"mime":"application/pdf"}})).await;
        let refused = next_text(&mut control).await;
        assert_eq!(refused["code"], "file_type");
        assert_eq!(refused["message"], "Tipo de arquivo não permitido");
        let id = Uuid::new_v4();
        emit(&mut control, json!({"v":1,"type":"request","id":id,"action":{"kind":"file.cancel","transferId":Uuid::new_v4()}})).await;
        assert_eq!(
            next_text(&mut control).await,
            json!({"v":1,"type":"ack","id":id})
        );
        let busy = Uuid::new_v4();
        offer(&mut control, busy, "a.pdf", 10, "application/pdf").await;
        let id = Uuid::new_v4();
        emit(&mut control, json!({"v":1,"type":"request","id":id,"action":{"kind":"file.offer","transferId":Uuid::new_v4(),"name":"b.pdf","size":10,"mime":"application/pdf"}})).await;
        let refused = next_text(&mut control).await;
        assert_eq!(refused["code"], "file_busy");
        assert_eq!(refused["retryable"], true);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn real_tls_file_transfer_aborts_delete_partial_data() {
        let (server, device, token) = file_server().await;
        let core = server.core.clone();
        let mut control = files_session(&server, device, &token).await;
        let folder = core.received_folder().unwrap();
        let pdf = b"%PDF-1.7\nconteudo".to_vec();

        // Content that does not match the declared type.
        let fake = b"MZ\x90\x00not a pdf".to_vec();
        let transfer = Uuid::new_v4();
        let ticket = offer(
            &mut control,
            transfer,
            "nota.pdf",
            fake.len(),
            "application/pdf",
        )
        .await;
        let mut upload = begin(&server, transfer, &ticket).await;
        upload
            .send(Message::Binary(fake.clone().into()))
            .await
            .unwrap();
        assert_eq!(next_text(&mut upload).await["received"], fake.len());
        let end = Uuid::new_v4();
        emit(&mut upload, json!({"v":1,"type":"file.end","id":end,"transferId":transfer,"sha256":hex::encode(Sha256::digest(&fake))})).await;
        let refused = next_text(&mut upload).await;
        assert_eq!(
            (refused["code"].as_str(), refused["id"].as_str()),
            (Some("file_type"), Some(end.to_string().as_str()))
        );
        assert!(part_files(&folder).is_empty());

        // Wrong digest.
        let transfer = Uuid::new_v4();
        let ticket = offer(
            &mut control,
            transfer,
            "nota.pdf",
            pdf.len(),
            "application/pdf",
        )
        .await;
        let mut upload = begin(&server, transfer, &ticket).await;
        upload
            .send(Message::Binary(pdf.clone().into()))
            .await
            .unwrap();
        next_text(&mut upload).await;
        emit(&mut upload, json!({"v":1,"type":"file.end","id":Uuid::new_v4(),"transferId":transfer,"sha256":"0".repeat(64)})).await;
        let refused = next_text(&mut upload).await;
        assert_eq!(refused["code"], "file_integrity");
        assert_eq!(refused["message"], "Arquivo corrompido durante o envio");
        assert!(part_files(&folder).is_empty());

        // More bytes than declared.
        let transfer = Uuid::new_v4();
        let ticket = offer(&mut control, transfer, "nota.pdf", 4, "application/pdf").await;
        let mut upload = begin(&server, transfer, &ticket).await;
        upload
            .send(Message::Binary(pdf.clone().into()))
            .await
            .unwrap();
        assert_eq!(next_text(&mut upload).await["code"], "too_large");
        assert!(part_files(&folder).is_empty());

        // A frame above chunkBytes is refused by the file socket limits.
        let transfer = Uuid::new_v4();
        let ticket = offer(
            &mut control,
            transfer,
            "nota.pdf",
            200_000,
            "application/pdf",
        )
        .await;
        let mut upload = begin(&server, transfer, &ticket).await;
        let _ = upload
            .send(Message::Binary(vec![b'%'; 70_000].into()))
            .await;
        // The unread payload can turn the server close into a reset; the error frame
        // is then lost, but the transfer must still be discarded.
        if let Ok(Some(Ok(Message::Text(text)))) =
            timeout(Duration::from_secs(5), upload.next()).await
        {
            assert_eq!(
                serde_json::from_str::<Value>(&text).unwrap()["code"],
                "invalid_message"
            );
        }
        wait_until(|| part_files(&folder).is_empty() && core.status().incoming_file.is_none())
            .await;

        // file.cancel from the phone closes the upload without an error frame.
        let transfer = Uuid::new_v4();
        let ticket = offer(&mut control, transfer, "nota.pdf", 100, "application/pdf").await;
        let mut upload = begin(&server, transfer, &ticket).await;
        upload
            .send(Message::Binary(b"%PDF-".to_vec().into()))
            .await
            .unwrap();
        received(&core, 5).await;
        let id = Uuid::new_v4();
        emit(&mut control, json!({"v":1,"type":"request","id":id,"action":{"kind":"file.cancel","transferId":transfer}})).await;
        assert_eq!(
            next_text(&mut control).await,
            json!({"v":1,"type":"ack","id":id})
        );
        assert_ne!(closed(&mut upload).await, Some(CloseCode::Normal));
        wait_until(|| part_files(&folder).is_empty()).await;

        // The phone closing the file socket early also discards the data.
        let transfer = Uuid::new_v4();
        let ticket = offer(&mut control, transfer, "nota.pdf", 100, "application/pdf").await;
        let mut upload = begin(&server, transfer, &ticket).await;
        upload
            .send(Message::Binary(b"%PDF-".to_vec().into()))
            .await
            .unwrap();
        received(&core, 5).await;
        assert_eq!(part_files(&folder).len(), 1);
        drop(upload);
        wait_until(|| part_files(&folder).is_empty() && core.status().incoming_file.is_none())
            .await;

        // The control session ending aborts an active upload.
        let transfer = Uuid::new_v4();
        let ticket = offer(&mut control, transfer, "nota.pdf", 100, "application/pdf").await;
        let mut upload = begin(&server, transfer, &ticket).await;
        upload
            .send(Message::Binary(b"%PDF-".to_vec().into()))
            .await
            .unwrap();
        received(&core, 5).await;
        control.close(None).await.unwrap();
        assert_eq!(next_text(&mut upload).await["code"], "unauthorized");
        assert!(part_files(&folder).is_empty());
        assert!(core.status().file_history.is_empty());
        assert!(std::fs::read_dir(&folder).unwrap().next().is_none());
        assert!(core
            .status()
            .diagnostics
            .iter()
            .all(|entry| !entry.message.contains("nota")
                && !entry.message.contains(&*folder.to_string_lossy())));
    }

    async fn receive(ws: &mut TestSocket, kind: &str) -> Value {
        timeout(Duration::from_secs(5), async {
            while let Some(message) = ws.next().await {
                if let Message::Text(text) = message.unwrap() {
                    let parsed: Value = serde_json::from_str(&text).unwrap();
                    if parsed["type"] == kind {
                        return parsed;
                    }
                }
            }
            panic!("Socket closed before expected response");
        })
        .await
        .expect("Expected response timeout")
    }

    async fn emit(ws: &mut TestSocket, value: Value) {
        ws.send(Message::Text(value.to_string().into()))
            .await
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn real_tls_websocket_pair_auth_revoke_and_cleanup() {
        let cert = rcgen::generate_simple_self_signed(vec!["pc-remote.local".into()]).unwrap();
        let certificate_pem = cert.cert.pem();
        let secrets = Secrets {
            pc_id: Uuid::new_v4(),
            certificate_pem: certificate_pem.clone(),
            private_key_pem: cert.signing_key.serialize_pem(),
            devices: Vec::new(),
        };
        let (config, fingerprint) =
            tls_config(&secrets.certificate_pem, &secrets.private_key_pem).unwrap();
        let core = Core::ephemeral(secrets, fingerprint);
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let server_core = core.clone();
        let server = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                let core = server_core.clone();
                tokio::spawn(async move {
                    if let Ok(tls) = acceptor.accept(stream).await {
                        let _ = handle(core, tls).await;
                    }
                });
            }
        });

        // An unrelated trust root fails TLS before any protocol credentials are sent.
        install_crypto_provider();
        let unrelated = rcgen::generate_simple_self_signed(vec!["pc-remote.local".into()]).unwrap();
        let mut roots = RootCertStore::empty();
        roots.add(unrelated.cert.der().clone()).unwrap();
        let bad_config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        assert!(TlsConnector::from(Arc::new(bad_config))
            .connect(ServerName::try_from("pc-remote.local").unwrap(), stream)
            .await
            .is_err());

        let mut unauthorized = connect(port, &certificate_pem).await;
        emit(
            &mut unauthorized,
            json!({"v":1,"type":"request","id":Uuid::new_v4(),"action":{"kind":"state.get"}}),
        )
        .await;
        assert_eq!(
            receive(&mut unauthorized, "error").await["code"],
            "unauthorized"
        );

        let device = Uuid::new_v4();
        let pair_id = Uuid::new_v4();
        let invitation = core.start_pairing();
        let mut wrong_code = connect(port, &certificate_pem).await;
        emit(
            &mut wrong_code,
            json!({"v":1,"type":"pair.request","id":Uuid::new_v4(),"deviceId":Uuid::new_v4(),"deviceName":"Wrong code test","code":"wrong-code"}),
        )
        .await;
        assert_eq!(
            receive(&mut wrong_code, "error").await["code"],
            "pairing_code_incorrect"
        );
        assert!(core.status().pending.is_empty());
        wrong_code.close(None).await.unwrap();

        let mut ws = connect(port, &certificate_pem).await;
        emit(&mut ws, json!({"v":1,"type":"pair.request","id":pair_id,"deviceId":device,"deviceName":"Integration test","code":invitation.code})).await;
        receive(&mut ws, "pair.pending").await;
        // Pings while waiting for local approval must not cancel the invitation.
        ws.send(Message::Ping(vec![1, 2, 3].into())).await.unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(core.status().pending.len(), 1);
        core.approve_pairing(pair_id, true).unwrap();
        let paired = receive(&mut ws, "pair.result").await;
        let token = paired["token"].as_str().unwrap().to_owned();
        emit(
            &mut ws,
            json!({"v":1,"type":"hello","id":Uuid::new_v4(),"deviceId":device,"token":token}),
        )
        .await;
        let legacy_welcome = receive(&mut ws, "welcome").await;
        assert_eq!(
            legacy_welcome["capabilities"],
            json!(["audio", "media", "clipboard.text"])
        );
        let state = receive(&mut ws, "state").await;
        assert!(state.get("text").is_none());
        assert!(state.get("input").is_none());
        assert!(state.get("files").is_none());
        let mut keys: Vec<&str> = state
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "audio",
                "clipboardRevision",
                "media",
                "revision",
                "sync",
                "type",
                "v"
            ]
        );
        // A legacy session never negotiated files.v1.
        let offer_id = Uuid::new_v4();
        emit(&mut ws, json!({"v":1,"type":"request","id":offer_id,"action":{"kind":"file.offer","transferId":Uuid::new_v4(),"name":"a.png","size":1,"mime":"image/png"}})).await;
        let refused = receive(&mut ws, "error").await;
        assert_eq!(
            (refused["code"].as_str(), refused["id"].as_str()),
            (Some("unavailable"), Some(offer_id.to_string().as_str()))
        );

        let id = Uuid::new_v4();
        let request = json!({"v":1,"type":"request","id":id,"action":{"kind":"state.get"}});
        emit(&mut ws, request.clone()).await;
        receive(&mut ws, "state").await;
        assert_eq!(receive(&mut ws, "ack").await["id"], id.to_string());
        emit(&mut ws, request).await;
        assert_eq!(receive(&mut ws, "error").await["code"], "invalid_message");

        let mut input_ws = connect_features(port, &certificate_pem, true).await;
        emit(
            &mut input_ws,
            json!({"v":1,"type":"hello","id":Uuid::new_v4(),"deviceId":device,"token":token}),
        )
        .await;
        let welcome = receive(&mut input_ws, "welcome").await;
        assert_eq!(
            welcome["capabilities"],
            json!(["audio", "media", "clipboard.text", "input.control"])
        );
        let state = receive(&mut input_ws, "state").await;
        // Allowed without a PC switch since 0.6; "Interromper" blocks it until "Liberar".
        assert_eq!(state["input"], json!({"allowed":true,"enabled":false}));
        assert!(state.get("files").is_none());
        core.stop_input_with_save(|_| Ok(())).unwrap();
        assert_eq!(
            receive(&mut input_ws, "state").await["input"],
            json!({"allowed":false,"enabled":false})
        );
        emit(&mut input_ws, json!({"v":1,"type":"request","id":Uuid::new_v4(),"action":{"kind":"input.enable","enabled":true}})).await;
        assert_eq!(receive(&mut input_ws, "error").await["code"], "unavailable");
        emit(&mut input_ws, json!({"v":1,"type":"request","id":Uuid::new_v4(),"action":{"kind":"pointer.move","dx":2,"dy":3}})).await;
        assert_eq!(receive(&mut input_ws, "error").await["code"], "unavailable");

        core.revoke(device).unwrap();
        assert!(!core.is_authorized_connection(device));
        let mut revoked = connect(port, &certificate_pem).await;
        emit(
            &mut revoked,
            json!({"v":1,"type":"hello","id":Uuid::new_v4(),"deviceId":device,"token":token}),
        )
        .await;
        assert_eq!(receive(&mut revoked, "error").await["code"], "unauthorized");

        let invitation = core.start_pairing();
        let pending_id = Uuid::new_v4();
        let mut pending = connect(port, &certificate_pem).await;
        emit(&mut pending, json!({"v":1,"type":"pair.request","id":pending_id,"deviceId":Uuid::new_v4(),"deviceName":"Dropped test","code":invitation.code})).await;
        receive(&mut pending, "pair.pending").await;
        pending.close(None).await.unwrap();
        drop(pending);
        timeout(Duration::from_secs(2), async {
            while !core.status().pending.is_empty() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        server.abort();
    }
}

fn execution_allowed(core: &Core, device: Uuid, epoch: u64) -> bool {
    core.check_resume();
    core.is_authorized_session(device, epoch)
}

async fn run_audio(
    core: Arc<Core>,
    device: Uuid,
    epoch: u64,
    percent: Option<u8>,
    step: Option<i8>,
    muted: Option<bool>,
) -> Result<(), String> {
    let checked = core.clone();
    let state = tokio::task::spawn_blocking(move || {
        if !execution_allowed(&checked, device, epoch) {
            return Err("unauthorized".into());
        }
        crate::audio::set(percent, step, muted)
    })
    .await
    .map_err(|_| "internal")?
    .map_err(|_| "unavailable")?;
    core.audio_changed(state);
    Ok(())
}

/// Runs an action. `Some` is a reply to send before the ack (the file ticket).
async fn dispatch(
    core: Arc<Core>,
    device: Uuid,
    epoch: u64,
    request_id: Uuid,
    action: Action,
) -> Result<Option<Value>, String> {
    if !execution_allowed(&core, device, epoch) {
        return Err("unauthorized".into());
    }
    let done: Result<(), String> = match action {
        Action::PowerRun {
            action,
            delay_seconds,
        } => {
            if let energy::Accepted::Now(action) =
                core.power_run(device, epoch, action, delay_seconds)?
            {
                // The ack goes out first; Windows acts half a second later.
                let later = core.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(energy::IMMEDIATE_DELAY).await;
                    later.power_execute(action);
                });
            }
            Ok(())
        }
        Action::PowerCancel {} => {
            core.power_cancel();
            Ok(())
        }
        Action::FileOffer {
            transfer_id,
            name,
            size,
            mime,
            clipboard,
        } => {
            let checked = core.clone();
            // The free-space check and folder preparation touch the disk.
            let ticket = tokio::task::spawn_blocking(move || {
                checked.file_offer(
                    device,
                    epoch,
                    transfer_id,
                    &name,
                    size,
                    &mime,
                    clipboard.as_ref(),
                )
            })
            .await
            .map_err(|_| "internal")??;
            return Ok(Some(json!({
                "v": 1,
                "type": "file.ticket",
                "id": request_id,
                "transferId": transfer_id,
                "ticket": ticket,
                "chunkBytes": protocol::FILE_CHUNK_BYTES,
                "windowBytes": protocol::FILE_WINDOW_BYTES,
            })));
        }
        Action::FileCancel { transfer_id } => {
            // The id names either an upload of the phone or a push of the PC.
            core.file_cancel(device, epoch, transfer_id);
            core.outgoing_refused(device, epoch, transfer_id, "Cancelado no celular.");
            Ok(())
        }
        Action::FileDecline { transfer_id, code } => {
            let reason = match code {
                DeclineCode::Storage => "Sem espaço no celular.",
                DeclineCode::Unavailable => "O celular não pôde receber agora.",
            };
            core.outgoing_refused(device, epoch, transfer_id, reason);
            Ok(())
        }
        Action::SearchOpen { service, query } => {
            let protocol::SearchService::Youtube = service;
            if !core.search_allowed() {
                return Err("rate_limited".into());
            }
            let query = protocol::search_query(&query)
                .map_err(str::to_string)?
                .to_string();
            let checked = core.clone();
            tokio::task::spawn_blocking(move || {
                if !execution_allowed(&checked, device, epoch) {
                    return Err("unauthorized".to_string());
                }
                crate::external::open_youtube_search(&query).map_err(|_| "unavailable".to_string())
            })
            .await
            .map_err(|_| "internal")?
        }
        Action::StateGet => Ok(()),
        Action::VolumeSet { percent } => {
            run_audio(core, device, epoch, Some(percent), None, None).await
        }
        Action::VolumeStep { delta } => {
            run_audio(core, device, epoch, None, Some(delta), None).await
        }
        Action::VolumeMute { muted } => {
            run_audio(core, device, epoch, None, None, Some(muted)).await
        }
        Action::MediaControl {
            session_id,
            command,
            offset_seconds,
        } => {
            let checked = core.clone();
            tokio::task::spawn_blocking(move || {
                crate::media::control(&session_id, &command, offset_seconds, move || {
                    execution_allowed(&checked, device, epoch)
                })
            })
            .await
            .map_err(|_| "internal")?
            .map_err(|_| "unavailable")?;
            core.media_changed();
            // Players apply commands asynchronously; publish their new state as soon as it shows.
            let later = core.clone();
            tokio::spawn(async move {
                for delay in [150, 350, 700] {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    let core = later.clone();
                    let _ = tokio::task::spawn_blocking(move || core.media_followup()).await;
                }
            });
            Ok(())
        }
        Action::ClipboardSet {
            event_id,
            text,
            automatic,
            base_revision,
            ..
        } => core
            .clipboard_set(event_id, &text, automatic, base_revision)
            .map_err(|e| {
                if [
                    "too_large",
                    "conflict",
                    "unavailable",
                    "clipboard_busy",
                    "invalid_message",
                ]
                .contains(&e.as_str())
                {
                    e
                } else {
                    "internal".into()
                }
            }),
        // An image may be read and converted, so it runs off the async workers.
        Action::ClipboardGet => {
            let checked = core.clone();
            tokio::task::spawn_blocking(move || checked.clipboard_get(device))
                .await
                .map_err(|_| "internal")?
                .map_err(|e| {
                    if ["too_large", "sensitive", "unavailable", "clipboard_busy"]
                        .contains(&e.as_str())
                    {
                        e
                    } else {
                        "internal".into()
                    }
                })
        }
        Action::SyncSet {
            upload,
            download,
            paused,
        } => {
            core.set_client_sync(upload, download, paused);
            Ok(())
        }
        Action::InputEnable { enabled } => core.input_enable(device, epoch, enabled),
        input @ (Action::PointerMove { .. }
        | Action::PointerClick { .. }
        | Action::PointerScroll { .. }
        | Action::KeyboardText { .. }
        | Action::KeyboardKey { .. }
        | Action::PresentationControl { .. }
        | Action::DeckTrigger { .. }) => {
            let checked = core.clone();
            tokio::task::spawn_blocking(move || {
                checked.check_resume();
                checked.execute_input(device, epoch, &input)
            })
            .await
            .map_err(|_| "internal")?
        }
    };
    done.map(|()| None)
}
