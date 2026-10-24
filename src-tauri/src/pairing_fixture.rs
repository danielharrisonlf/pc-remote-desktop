//! Local-only, real-server pairing fixture used by the Android QA flow.
//!
//! Run with `PCREMOTE_PAIRING_QA_DIR=<absolute artifacts dir>` and
//! `cargo test pairing_fixture::real_pairing_fixture -- --ignored --nocapture`.
//! The fixture accepts one JSON object per line in `commands.jsonl`:
//! `{"op":"startA"}`, `{"op":"startB"}`,
//! `{"op":"approvefirstpendingA","approved":true}` (and B),
//! `{"op":"expire_invitationA"}` (and B),
//! `{"op":"expire_requestA"}` (and B), or `{"op":"stop"}`.
//! Creating the `stop` file also shuts it down. It always stops after 15 minutes.

use crate::{
    connection::{self, tls_config},
    core::Core,
    secrets::{Device, Secrets},
    settings::Settings,
};
use mdns_sd::{IfKind, Receiver as MdnsReceiver, ServiceDaemon, ServiceEvent, ServiceInfo};
use rcgen::generate_simple_self_signed;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    net::TcpListener,
    sync::watch,
    task::{JoinHandle, JoinSet},
    time::timeout,
};
use tokio_rustls::TlsAcceptor;
use uuid::Uuid;

const FIXTURE_MAX_RUNTIME: Duration = Duration::from_secs(15 * 60);
const FIXTURE_HOST: &str = "10.0.2.2";
const MDNS_SERVICE: &str = "_pcremote._tcp.local.";

#[derive(Clone, Serialize)]
struct PublicInvite {
    v: u8,
    host: String,
    port: u16,
    #[serde(rename = "pcId")]
    pc_id: Uuid,
    fingerprint: String,
    code: String,
}

struct FixturePc {
    name: &'static str,
    core: Arc<Core>,
    invite: PublicInvite,
    shutdown: watch::Sender<bool>,
    server_task: JoinHandle<()>,
}

#[derive(Deserialize)]
struct FixtureCommand {
    op: String,
    approved: Option<bool>,
}

#[derive(Default, Serialize)]
struct MdnsReport {
    daemon_started: bool,
    registered: Vec<String>,
    expected_pc_ids: Vec<String>,
    resolved: Vec<MdnsResolved>,
    both_expected_instances_resolved: bool,
    reachability_note: Option<String>,
    error: Option<String>,
}

#[derive(Clone, Serialize)]
struct MdnsResolved {
    fullname: String,
    host: String,
    port: u16,
    pc_id: Option<String>,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "starts two local TLS/WebSocket servers for interactive Android QA"]
async fn real_pairing_fixture() {
    let Some(dir) = std::env::var_os("PCREMOTE_PAIRING_QA_DIR").map(PathBuf::from) else {
        eprintln!("Set PCREMOTE_PAIRING_QA_DIR to an artifacts directory to run this fixture.");
        return;
    };
    if !dir.is_absolute() {
        panic!("PCREMOTE_PAIRING_QA_DIR must be an absolute path");
    }
    fs::create_dir_all(&dir).expect("create QA artifact directory");
    let stop_file = dir.join("stop");
    if stop_file.exists() {
        panic!("Remove the stale stop file before starting the fixture");
    }

    let mut a = start_pc("PC-QA-A").await.expect("start fixture PC A");
    let mut b = match start_pc("PC-QA-B").await {
        Ok(pc) => pc,
        Err(error) => {
            stop_pc(&mut a).await;
            panic!("start fixture PC B: {error}");
        }
    };
    assert_ne!(a.invite.pc_id, b.invite.pc_id);
    assert_ne!(a.invite.fingerprint, b.invite.fingerprint);

    let (mdns, mdns_events, mut mdns_report) = start_mdns(&a, &b);
    write_json(
        &dir.join("invites.json"),
        &json!({"A":a.invite,"B":b.invite}),
    )
    .expect("write public fixture invitations");
    write_json(&dir.join("mdns.json"), &mdns_report).expect("write initial mDNS report");
    write_json(
        &dir.join("ready.json"),
        &json!({
            "ready": true,
            "startedAtUnixMs": unix_ms(),
            "maxRuntimeSeconds": FIXTURE_MAX_RUNTIME.as_secs(),
            "inviteFile": "invites.json",
            "statusFile": "status.json",
            "commandsFile": "commands.jsonl",
            "stopFile": "stop",
            "servers": ["PC-QA-A", "PC-QA-B"],
            "mDNSBrowse": "_pcremote._tcp.local."
        }),
    )
    .expect("write fixture ready marker");

    let deadline = Instant::now() + FIXTURE_MAX_RUNTIME;
    let commands_path = dir.join("commands.jsonl");
    let mut command_offset = 0usize;
    let mut command_results: Vec<Value> = Vec::new();
    let mut stop_reason = "timeout";
    let mut stop_requested = false;
    loop {
        if stop_file.exists() {
            stop_reason = "stop_file";
            break;
        }
        if Instant::now() >= deadline {
            break;
        }

        if let Ok(contents) = fs::read(&commands_path) {
            if contents.len() < command_offset {
                command_offset = 0;
            }
            let new_bytes = &contents[command_offset..];
            if let Some(last_newline) = new_bytes.iter().rposition(|byte| *byte == b'\n') {
                let batch = &new_bytes[..=last_newline];
                for raw_line in batch.split(|byte| *byte == b'\n') {
                    if raw_line.is_empty() {
                        continue;
                    }
                    let result = match serde_json::from_slice::<FixtureCommand>(raw_line) {
                        Ok(command) => {
                            let is_stop = command.op == "stop";
                            let result = execute_command(command, &mut a, &mut b);
                            stop_requested |= is_stop;
                            result
                        }
                        Err(_) => json!({"ok":false,"error":"invalid_command_json"}),
                    };
                    command_results.push(result);
                }
                command_offset += last_newline + 1;
            }
        }
        if stop_requested {
            stop_reason = "command";
            break;
        }

        if let Some(events) = &mdns_events {
            collect_mdns(events, &mut mdns_report);
            let _ = write_json(&dir.join("mdns.json"), &mdns_report);
        }
        let _ = write_json(
            &dir.join("invites.json"),
            &json!({"A":a.invite,"B":b.invite}),
        );
        let _ = write_json(
            &dir.join("status.json"),
            &public_status(&a, &b, &command_results),
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let _ = write_json(
        &dir.join("status.json"),
        &public_status(&a, &b, &command_results),
    );
    let _ = write_json(
        &dir.join("stopped.json"),
        &json!({
            "stoppedAtUnixMs": unix_ms(),
            "reason": stop_reason,
            "runtimeSeconds": FIXTURE_MAX_RUNTIME.as_secs().saturating_sub(deadline.saturating_duration_since(Instant::now()).as_secs()),
            "mdns": mdns_report
        }),
    );
    if let Some(daemon) = mdns {
        let _ = daemon.shutdown();
    }
    stop_pc(&mut a).await;
    stop_pc(&mut b).await;
}

async fn start_pc(name: &'static str) -> Result<FixturePc, String> {
    let identity = generate_simple_self_signed(vec!["pc-remote.local".to_owned()])
        .map_err(|error| error.to_string())?;
    let secrets = Secrets {
        pc_id: Uuid::new_v4(),
        certificate_pem: identity.cert.pem(),
        private_key_pem: identity.signing_key.serialize_pem(),
        devices: Vec::<Device>::new(),
    };
    let (tls_config, fingerprint) = tls_config(&secrets.certificate_pem, &secrets.private_key_pem)?;
    let core = Core::ephemeral(secrets, fingerprint.clone());
    {
        let mut state = core
            .state
            .lock()
            .map_err(|_| "fixture Core state poisoned")?;
        state.pc_name = name.to_owned();
        state.settings = Settings {
            realtime: false,
            autostart: false,
            input_blocked: true,
        };
    }
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    core.set_service_running(true);
    let acceptor = TlsAcceptor::from(Arc::new(tls_config));
    let server_core = core.clone();
    let (shutdown, shutdown_rx) = watch::channel(false);
    let server_task = tokio::spawn(async move {
        run_listener(listener, server_core, acceptor, shutdown_rx).await;
    });
    let pairing = core.start_pairing();
    let status = core.status();
    let invite = PublicInvite {
        v: 1,
        host: FIXTURE_HOST.to_owned(),
        port: address.port(),
        pc_id: status.pc_id,
        fingerprint,
        code: pairing.code,
    };
    Ok(FixturePc {
        name,
        core,
        invite,
        shutdown,
        server_task,
    })
}

async fn run_listener(
    listener: TcpListener,
    core: Arc<Core>,
    acceptor: TlsAcceptor,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { break; }
            }
            accepted = listener.accept() => {
                let Ok((stream, _peer)) = accepted else { break; };
                let _ = stream.set_nodelay(true);
                let connection_core = core.clone();
                let connection_acceptor = acceptor.clone();
                connections.spawn(async move {
                    if let Ok(Ok(tls)) = timeout(Duration::from_secs(8), connection_acceptor.accept(stream)).await {
                        let _ = timeout(Duration::from_secs(3600), connection::handle(connection_core, tls)).await;
                    }
                });
            }
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
        }
    }
    connections.abort_all();
    while connections.join_next().await.is_some() {}
}

fn start_mdns(
    a: &FixturePc,
    b: &FixturePc,
) -> (
    Option<ServiceDaemon>,
    Option<MdnsReceiver<ServiceEvent>>,
    MdnsReport,
) {
    let mut report = MdnsReport::default();
    let daemon = match ServiceDaemon::new() {
        Ok(daemon) => daemon,
        Err(_) => {
            report.error = Some("daemon_unavailable".into());
            return (None, None, report);
        }
    };
    report.daemon_started = true;
    report.reachability_note = Some(
        "Browse/resolution uses loopback records; Android connects through the public invite host 10.0.2.2 because listeners bind only to 127.0.0.1.".into(),
    );
    report.expected_pc_ids = vec![a.invite.pc_id.to_string(), b.invite.pc_id.to_string()];
    let _ = daemon.disable_interface(IfKind::IPv6);
    let _ = daemon.enable_interface(IfKind::LoopbackV4);
    let events = match daemon.browse(MDNS_SERVICE) {
        Ok(events) => events,
        Err(_) => {
            report.error = Some("browse_unavailable".into());
            return (Some(daemon), None, report);
        }
    };
    for pc in [a, b] {
        let pc_id = pc.invite.pc_id.to_string();
        let properties = [("id", pc_id.as_str()), ("v", "1")];
        let info = ServiceInfo::new(
            MDNS_SERVICE,
            pc.name,
            &format!("{pc_id}.local."),
            (),
            pc.invite.port,
            &properties[..],
        );
        match info {
            Ok(mut info) => {
                info = info.enable_addr_auto();
                info.set_interfaces(vec![IfKind::LoopbackV4]);
                if daemon.register(info).is_ok() {
                    report.registered.push(pc.name.to_owned());
                } else {
                    report.error = Some("registration_failed".into());
                }
            }
            _ => report.error = Some("registration_failed".into()),
        }
    }
    (Some(daemon), Some(events), report)
}

fn collect_mdns(events: &MdnsReceiver<ServiceEvent>, report: &mut MdnsReport) {
    while let Ok(event) = events.try_recv() {
        if let ServiceEvent::ServiceResolved(service) = event {
            let resolved = MdnsResolved {
                fullname: service.fullname.clone(),
                host: service.host.clone(),
                port: service.port,
                pc_id: service.get_property_val_str("id").map(str::to_owned),
            };
            if !report
                .resolved
                .iter()
                .any(|item| item.fullname == resolved.fullname)
            {
                report.resolved.push(resolved);
                report
                    .resolved
                    .sort_by(|left, right| left.fullname.cmp(&right.fullname));
            }
        }
    }
    report.both_expected_instances_resolved = report.expected_pc_ids.iter().all(|expected| {
        report
            .resolved
            .iter()
            .any(|resolved| resolved.pc_id.as_ref() == Some(expected))
    });
}

fn execute_command(command: FixtureCommand, a: &mut FixturePc, b: &mut FixturePc) -> Value {
    match command.op.as_str() {
        "stop" => json!({"ok":true,"op":"stop","note":"create the stop file to end the fixture"}),
        "startA" => restart_invitation(a),
        "startB" => restart_invitation(b),
        "approvefirstpendingA" => approve_first(a, command.approved),
        "approvefirstpendingB" => approve_first(b, command.approved),
        "expire_invitationA" => expire_invitation(a),
        "expire_invitationB" => expire_invitation(b),
        "expire_requestA" => expire_request(a),
        "expire_requestB" => expire_request(b),
        _ => json!({"ok":false,"error":"unsupported_operation"}),
    }
}

fn restart_invitation(pc: &mut FixturePc) -> Value {
    let pairing = pc.core.start_pairing();
    pc.invite.code = pairing.code;
    pc.invite.pc_id = pc.core.status().pc_id;
    json!({"ok":true,"op":format!("start{}", pc.name.ends_with('A').then_some('A').unwrap_or('B')),"pcId":pc.invite.pc_id})
}

fn approve_first(pc: &FixturePc, approved: Option<bool>) -> Value {
    let Some(approved) = approved else {
        return json!({"ok":false,"error":"approved_boolean_required"});
    };
    let Some(id) = pc
        .core
        .status()
        .pending
        .into_iter()
        .min_by_key(|pending| (pending.requested_at, pending.id))
        .map(|pending| pending.id)
    else {
        return json!({"ok":false,"error":"no_pending_request"});
    };
    match pc.core.approve_pairing(id, approved) {
        Ok(()) => {
            json!({"ok":true,"op":format!("approvefirstpending{}", if pc.name.ends_with('A') {"A"} else {"B"}),"approved":approved,"requestId":id})
        }
        Err(error) => json!({"ok":false,"op":"approvefirstpending","requestId":id,"error":error}),
    }
}

fn expire_invitation(pc: &FixturePc) -> Value {
    let mut state = pc.core.state.lock().unwrap();
    let Some((_, deadline)) = state.pairing.as_mut() else {
        return json!({"ok":false,"error":"no_invitation"});
    };
    *deadline = Instant::now() - Duration::from_secs(1);
    json!({"ok":true,"op":format!("expire_invitation{}", if pc.name.ends_with('A') {"A"} else {"B"})})
}

fn expire_request(pc: &FixturePc) -> Value {
    let Some(id) = pc
        .core
        .status()
        .pending
        .into_iter()
        .min_by_key(|pending| (pending.requested_at, pending.id))
        .map(|pending| pending.id)
    else {
        return json!({"ok":false,"error":"no_pending_request"});
    };
    {
        let mut state = pc.core.state.lock().unwrap();
        let Some(pending) = state.pending.get_mut(&id) else {
            return json!({"ok":false,"error":"pending_request_disappeared"});
        };
        pending.deadline = Instant::now() - Duration::from_secs(1);
        pending.expires_at = 0;
    }
    let _ = pc.core.expire_pending(id);
    json!({"ok":true,"op":format!("expire_request{}", if pc.name.ends_with('A') {"A"} else {"B"}),"requestId":id})
}

fn public_status(a: &FixturePc, b: &FixturePc, commands: &[Value]) -> Value {
    fn pc_status(pc: &FixturePc) -> Value {
        let status = pc.core.status();
        json!({
            "name": pc.name,
            "pcId": status.pc_id,
            "port": pc.invite.port,
            "fingerprint": status.fingerprint,
            "settings": status.settings,
            "pending": status.pending,
            "devices": status.devices,
            "pairingResult": status.pairing_result,
            "invitationExpired": pc.core.state.lock().unwrap().pairing.as_ref().is_some_and(|(_,deadline)| *deadline <= Instant::now())
        })
    }
    json!({"updatedAtUnixMs":unix_ms(),"pcs":{"A":pc_status(a),"B":pc_status(b)},"commands":commands})
}

fn write_json(path: &Path, value: &impl Serialize) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(std::io::Error::other)?;
    let tmp_path = path.with_extension("tmp");
    fs::write(&tmp_path, bytes)?;
    fs::rename(tmp_path, path)
}

async fn stop_pc(pc: &mut FixturePc) {
    let _ = pc.shutdown.send(true);
    let _ = timeout(Duration::from_secs(2), &mut pc.server_task).await;
    pc.core.set_service_running(false);
}

fn unix_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
