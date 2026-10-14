use crate::{
    core::{Core, Outgoing},
    protocol::{self, Action, ClientMessage},
};
use futures_util::{SinkExt, StreamExt};
use mdns_sd::{ServiceDaemon, ServiceInfo};
use rustls::{
    pki_types::{CertificateDer, PrivateKeyDer},
    ServerConfig,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{io::Cursor, sync::Arc, time::Duration};
use tokio::{net::TcpListener, sync::Semaphore, time::timeout};
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::{
    accept_hdr_async_with_config,
    tungstenite::{
        handshake::server::{ErrorResponse, Request, Response},
        protocol::WebSocketConfig,
        Message,
    },
};
use uuid::Uuid;

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

async fn send(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
    >,
    v: Value,
) -> Result<(), String> {
    ws.send(Message::Text(v.to_string().into()))
        .await
        .map_err(|e| e.to_string())
}
async fn error(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
    >,
    id: Uuid,
    code: &str,
) -> Result<(), String> {
    let (message, retryable) = match code {
        "version_mismatch" => ("Versão do protocolo incompatível", false),
        "unauthorized" => ("Dispositivo sem autorização", false),
        "pairing_expired" => ("Código inválido ou expirado", false),
        "pairing_denied" => ("Pareamento recusado no PC", false),
        "rate_limited" => ("Muitas tentativas. Tente depois.", true),
        "unavailable" => ("Ação indisponível neste dispositivo ou player", false),
        "conflict" => ("Houve outra cópia antes desta", false),
        "sensitive" => ("Conteúdo marcado como sensível", false),
        "too_large" => ("Texto excede 64 KiB", false),
        "clipboard_busy" => ("Área de transferência ocupada", true),
        "invalid_message" => ("Mensagem inválida", false),
        _ => ("Falha ao executar ação", true),
    };
    send(ws, protocol::wire_error(id, code, message, retryable)).await
}

async fn handle(
    core: Arc<Core>,
    tls: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
) -> Result<(), String> {
    let mut config = WebSocketConfig::default();
    config.max_message_size = Some(protocol::MAX_FRAME);
    config.max_frame_size = Some(protocol::MAX_FRAME);
    let mut ws = timeout(
        Duration::from_secs(8),
        accept_hdr_async_with_config(
            tls,
            |req: &Request, response: Response| -> Result<Response, ErrorResponse> {
                if req.uri().path() != "/v1/ws" || req.headers().contains_key("origin") {
                    return Err(tungstenite_forbidden());
                }
                Ok(response)
            },
            Some(config),
        ),
    )
    .await
    .map_err(|_| "WebSocket upgrade timeout")?
    .map_err(|e| e.to_string())?;
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
                if id.is_nil()||seen.len()>=65_536{error(&mut ws,id,"invalid_message").await?;break;}
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
                        let epoch = match core.connect_authenticated(device_id,&token) { Ok(epoch) => epoch, Err(code) => {error(&mut ws,id,code).await?;break;} };
                        events = core.events.subscribe();
                        first_valid_message=true;device=Some(device_id);cleanup.device=device;cleanup.epoch=epoch;let status=core.status();
                        send(&mut ws,json!({"v":1,"type":"welcome","id":id,"pcId":status.pc_id,"pcName":status.pc_name,"capabilities":["audio","media","clipboard.text"]})).await?;
                        send(&mut ws,serde_json::to_value(core.snapshot()).map_err(|e|e.to_string())?).await?;
                    }
                    ClientMessage::Request {action,..} if device.is_some()=>{
                        if !core.is_authorized_session(device.unwrap(),cleanup.epoch){error(&mut ws,id,"unauthorized").await?;break;}
                        if let Err(code)=protocol::validate_action(&action){error(&mut ws,id,code).await?;continue;}
                        let result=dispatch(core.clone(),device.unwrap(),cleanup.epoch,action).await;
                        match result {Ok(())=>{send(&mut ws,serde_json::to_value(core.snapshot()).map_err(|e|e.to_string())?).await?;send(&mut ws,protocol::ack(id)).await?},Err(code)=>error(&mut ws,id,&code).await?}
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

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::secrets::Secrets;
    use rustls::{pki_types::ServerName, ClientConfig, RootCertStore};
    use tokio_rustls::TlsConnector;
    type TestSocket =
        tokio_tungstenite::WebSocketStream<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>;

    async fn connect(port: u16, cert: &str) -> TestSocket {
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
        tokio_tungstenite::client_async("wss://pc-remote.local/v1/ws", tls)
            .await
            .unwrap()
            .0
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
        receive(&mut ws, "welcome").await;
        let state = receive(&mut ws, "state").await;
        assert!(state.get("text").is_none());

        let id = Uuid::new_v4();
        let request = json!({"v":1,"type":"request","id":id,"action":{"kind":"state.get"}});
        emit(&mut ws, request.clone()).await;
        receive(&mut ws, "state").await;
        assert_eq!(receive(&mut ws, "ack").await["id"], id.to_string());
        emit(&mut ws, request).await;
        assert_eq!(receive(&mut ws, "error").await["code"], "invalid_message");

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

async fn dispatch(core: Arc<Core>, device: Uuid, epoch: u64, action: Action) -> Result<(), String> {
    if !execution_allowed(&core, device, epoch) {
        return Err("unauthorized".into());
    }
    match action {
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
    }
}
