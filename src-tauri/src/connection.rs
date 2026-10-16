use crate::{
    core::{Core, Outgoing},
    files::{self, Accepted},
    protocol::{self, Action, ClientMessage, Features, FileClientMessage},
};
use futures_util::{SinkExt, StreamExt};
use mdns_sd::{ServiceDaemon, ServiceInfo};
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

const FILE_BEGIN_TIMEOUT: Duration = Duration::from_secs(10);
const FILE_IDLE_TIMEOUT: Duration = Duration::from_secs(20);
const FILE_TOTAL_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const FILE_PROGRESS_STEP: u64 = 256 * 1024;
const FILE_HEAD_BYTES: usize = 1024;
/// Chunks queued for the blocking writer; matches the 1 MiB client window.
const FILE_QUEUE_CHUNKS: usize = protocol::FILE_WINDOW_BYTES / protocol::FILE_CHUNK_BYTES;

pub fn tls_config(certificate_pem: &str, key_pem: &str) -> Result<(ServerConfig, String), String> {
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
    let mdns = ServiceDaemon::new().ok();
    if mdns.is_none() {
        core.diagnostic("Descoberta mDNS indisponível. Use o endereço manual.");
    }
    let status = core.status();
    if let Ok(info) = ServiceInfo::new(
        "_pcremote._tcp.local.",
        &status.pc_name,
        &format!("{}.local.", status.pc_id),
        (),
        protocol::PORT,
        &[("id", status.pc_id.to_string().as_str()), ("v", "1")][..],
    ) {
        if let Some(mdns) = &mdns {
            let _ = mdns.register(info.enable_addr_auto());
        }
    }
    let acceptor = TlsAcceptor::from(Arc::new(config));
    core.set_service_running(true);
    let slots = Arc::new(Semaphore::new(16));
    loop {
        let (stream, _) = listener.accept().await.map_err(|e| e.to_string())?;
        let Ok(permit) = slots.clone().try_acquire_owned() else {
            continue;
        };
        let core = core.clone();
        let acceptor = acceptor.clone();
        tokio::spawn(async move {
            let _permit = permit;
            if let Ok(Ok(tls)) = timeout(Duration::from_secs(8), acceptor.accept(stream)).await {
                let _ = timeout(Duration::from_secs(3600), handle(core, tls)).await;
            }
        });
    }
}

async fn send(ws: &mut ServerSocket, v: Value) -> Result<(), String> {
    ws.send(Message::Text(v.to_string().into()))
        .await
        .map_err(|e| e.to_string())
}
async fn error(ws: &mut ServerSocket, id: Uuid, code: &str) -> Result<(), String> {
    reject(ws, id, code, false).await
}
/// Error frame; `file` selects the file-transfer wording for shared codes.
async fn reject(ws: &mut ServerSocket, id: Uuid, code: &str, file: bool) -> Result<(), String> {
    let (message, retryable) = protocol::error_text(code, file);
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

async fn handle(
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
            _=heartbeat.tick()=>{ws.send(Message::Ping(Vec::new().into())).await.map_err(|e|e.to_string())?;}
            incoming=ws.next()=>{
                core.check_resume();
                let Some(Ok(msg))=incoming else {break};
                if msg.is_close(){break;}
                if msg.is_ping(){ws.send(Message::Pong(msg.into_data())).await.map_err(|e|e.to_string())?;continue;}
                let Message::Text(raw)=msg else {continue};
                if raw.len()>protocol::MAX_FRAME{let _=error(&mut ws,Uuid::nil(),"too_large").await;break;}
                let parsed=serde_json::from_str::<ClientMessage>(&raw);
                let msg=match parsed {Ok(m)=>m,Err(_)=>{error(&mut ws,Uuid::nil(),"invalid_message").await?;continue}};
                let id=msg.id();
                if msg.version()!=1{error(&mut ws,id,"version_mismatch").await?;continue;}
                if id.is_nil()||seen.len()>=262_144{error(&mut ws,id,"invalid_message").await?;break;}
                if !seen.insert(id){error(&mut ws,id,"invalid_message").await?;continue;}
                match msg {
                    ClientMessage::PairRequest {device_id,device_name,code,..} if device.is_none()=>{
                        match core.pair_request(id,device_id,device_name.clone(),code){
                            Ok((receiver,generation))=>{
                                first_valid_message=true;
                                struct PendingCleanup {core:Arc<Core>,id:Uuid}
                                impl Drop for PendingCleanup {fn drop(&mut self){self.core.cancel_pending(self.id);}}
                                let _pending=PendingCleanup{core:core.clone(),id};
                                send(&mut ws,json!({"v":1,"type":"pair.pending","id":id})).await?;
                                let mut receiver=receiver;
                                let deadline=tokio::time::Instant::now()+Duration::from_secs(120);
                                let approved=loop {tokio::select!{
                                    answer=&mut receiver=>break answer.unwrap_or(false),
                                    _=tokio::time::sleep_until(deadline)=>break false,
                                    incoming=ws.next()=>match incoming{
                                        Some(Ok(Message::Ping(payload)))=>ws.send(Message::Pong(payload)).await.map_err(|e|e.to_string())?,
                                        Some(Ok(Message::Pong(_)))=>{},
                                        _=>return Ok(())
                                    }
                                }};
                                if approved {match core.pair_complete(device_id,device_name,generation){Ok((pc_id,token))=>{let status=core.status();send(&mut ws,json!({"v":1,"type":"pair.result","id":id,"pcId":pc_id,"pcName":status.pc_name,"token":token})).await?},Err(code)=>error(&mut ws,id,&code).await?}}
                                else {error(&mut ws,id,"pairing_denied").await?}
                            },Err(code)=>error(&mut ws,id,code).await?
                        }
                    }
                    ClientMessage::Hello {device_id,token,..} if device.is_none()=>{
                        let epoch = match core.connect_authenticated(device_id,&token,features.input,features.files) { Ok(epoch) => epoch, Err(code) => {error(&mut ws,id,code).await?;break;} };
                        events = core.events.subscribe();
                        first_valid_message=true;device=Some(device_id);cleanup.device=device;cleanup.epoch=epoch;let status=core.status();
                        let mut capabilities = vec!["audio", "media", "clipboard.text"];
                        if features.input { capabilities.push("input.control"); }
                        if features.files { capabilities.push("file.transfer"); }
                        send(&mut ws,json!({"v":1,"type":"welcome","id":id,"pcId":status.pc_id,"pcName":status.pc_name,"capabilities":capabilities})).await?;
                        send(&mut ws,serde_json::to_value(core.snapshot()).map_err(|e|e.to_string())?).await?;
                    }
                    ClientMessage::Request {action,..} if device.is_some()=>{
                        if !core.is_authorized_session(device.unwrap(),cleanup.epoch){error(&mut ws,id,"unauthorized").await?;break;}
                        let file = action.is_file();
                        if file {if let Err(code)=core.file_gate(device.unwrap(),cleanup.epoch,matches!(&action, Action::FileOffer { .. })){reject(&mut ws,id,code,true).await?;continue;}}
                        if let Err(code)=protocol::validate_action(&action){reject(&mut ws,id,code,file).await?;continue;}
                        let send_snapshot = !(action.is_input() || file) || matches!(&action, Action::InputEnable { .. });
                        let result=dispatch(core.clone(),device.unwrap(),cleanup.epoch,id,action).await;
                        match result {Ok(reply)=>{if let Some(reply)=reply {send(&mut ws,reply).await?;}if send_snapshot {send(&mut ws,serde_json::to_value(core.snapshot()).map_err(|e|e.to_string())?).await?;}send(&mut ws,protocol::ack(id)).await?},Err(code)=>reject(&mut ws,id,&code,file).await?}
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
    fn diagnostic(&self) -> String {
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
    if message.id().is_nil() {
        return Err((Uuid::nil(), "invalid_message"));
    }
    Ok(message)
}

async fn close_file_socket(ws: &mut ServerSocket, reply: Option<(Uuid, &str)>, done: bool) {
    if let Some((id, code)) = reply {
        let _ = reject(ws, id, code, true).await;
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

/// `/v1/files`: one ticket-authorized upload per socket.
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
            core.diagnostic(&abort.diagnostic());
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
        Ok((end_id, name)) => {
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
            core.diagnostic(&abort.diagnostic());
            close_file_socket(&mut ws, abort.reply(), false).await;
        }
    }
    Ok(())
}

/// Streams binary frames to a blocking writer, then verifies and saves the file.
async fn receive_file(
    core: &Core,
    ws: &mut ServerSocket,
    accepted: &Accepted,
    file: std::fs::File,
    part: &std::path::Path,
) -> Result<(Uuid, String), FileAbort> {
    let (chunks, queue) = mpsc::channel::<Bytes>(FILE_QUEUE_CHUNKS);
    let writer = tokio::task::spawn_blocking(move || files::write_part(file, queue));
    let mut head = Vec::with_capacity(FILE_HEAD_BYTES);
    let mut received = 0u64;
    let mut reported = 0u64;
    let total_deadline = Instant::now() + FILE_TOTAL_TIMEOUT;
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
        Ok(Ok(saved)) => Ok((end_id, saved)),
        Ok(Err(error)) => Err(FileAbort::Storage(end_id, error)),
        Err(_) => Err(FileAbort::Reply(end_id, "internal")),
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
            s.settings.allow_files = true;
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

    #[test]
    fn feature_header_tokens_from_every_line() {
        use tokio_tungstenite::tungstenite::http::HeaderValue;
        let parse = |values: &[&str]| {
            let values: Vec<HeaderValue> = values.iter().map(|v| v.parse().unwrap()).collect();
            Features::parse(values.iter().map(|v| v.as_bytes()))
        };
        assert_eq!(parse(&[]), Features::default());
        assert_eq!(
            parse(&["input.v1"]),
            Features {
                input: true,
                files: false
            }
        );
        assert_eq!(
            parse(&["input.v1", "files.v1"]),
            Features {
                input: true,
                files: true
            }
        );
        assert_eq!(
            parse(&["input.v1, files.v1"]),
            Features {
                input: true,
                files: true
            }
        );
        assert_eq!(
            parse(&["files.v1,input.v1"]),
            Features {
                input: true,
                files: true
            }
        );
        assert_eq!(
            parse(&["  files.v1\t"]),
            Features {
                input: false,
                files: true
            }
        );
        assert_eq!(
            parse(&["files.v2", "input.v1x", "INPUT.V1"]),
            Features::default()
        );
        assert_eq!(parse(&["files.v1;q=1"]), Features::default());
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
        assert_eq!(
            std::fs::read(folder.join("IMG 0001 (1).png:Zone.Identifier")).unwrap(),
            b"[ZoneTransfer]\r\nZoneId=3\r\n"
        );
        assert!(part_files(&folder).is_empty());
        let status = core.status();
        assert!(status.incoming_file.is_none());
        assert_eq!(status.received_files.len(), 1);
        assert_eq!(status.received_files[0].name, "IMG 0001 (1).png");
        assert_eq!(status.received_files[0].bytes, payload.len() as u64);
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

        // Switching allowFiles off aborts the transfer and publishes the new state.
        let transfer = Uuid::new_v4();
        let ticket = offer(&mut control, transfer, "nota.pdf", 100, "application/pdf").await;
        let mut upload = begin(&server, transfer, &ticket).await;
        upload
            .send(Message::Binary(b"%PDF-".to_vec().into()))
            .await
            .unwrap();
        received(&core, 5).await;
        let mut settings = core.status().settings;
        settings.allow_files = false;
        core.update_settings_with_io(settings, |_| Ok(()), |_| Ok(()))
            .unwrap();
        assert_eq!(next_text(&mut upload).await["code"], "unavailable");
        assert_eq!(
            receive(&mut control, "state").await["files"]["allowed"],
            false
        );
        assert!(part_files(&folder).is_empty());
        let id = Uuid::new_v4();
        emit(&mut control, json!({"v":1,"type":"request","id":id,"action":{"kind":"file.offer","transferId":Uuid::new_v4(),"name":"a.pdf","size":10,"mime":"application/pdf"}})).await;
        assert_eq!(next_text(&mut control).await["code"], "unavailable");

        // The control session ending aborts an active upload.
        core.state.lock().unwrap().settings.allow_files = true;
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
        assert!(core.status().received_files.is_empty());
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
        assert_eq!(state["input"], json!({"allowed":false,"enabled":false}));
        assert!(state.get("files").is_none());
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
        Action::FileOffer {
            transfer_id,
            name,
            size,
            mime,
        } => {
            let checked = core.clone();
            // The free-space check and folder preparation touch the disk.
            let ticket = tokio::task::spawn_blocking(move || {
                checked.file_offer(device, epoch, transfer_id, &name, size, &mime)
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
            core.file_cancel(device, epoch, transfer_id);
            Ok(())
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
        Action::ClipboardGet => core.clipboard_get(device).map_err(|e| {
            if ["too_large", "sensitive", "unavailable", "clipboard_busy"].contains(&e.as_str()) {
                e
            } else {
                "internal".into()
            }
        }),
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
        | Action::KeyboardKey { .. }) => {
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
