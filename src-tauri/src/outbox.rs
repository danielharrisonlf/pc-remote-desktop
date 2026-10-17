//! PC → phone files (files.v2).
//!
//! The panel queues files chosen in a dialog or dropped on the window. One at a
//! time is announced with `file.push` on the control socket; the phone redeems
//! the single-use ticket on `/v1/files` with `file.fetch` and confirms what it
//! wrote with `file.progress`. Nothing is retried automatically.
//!
//! clipboard.v2 images ride the same queue as temporary PNG files: ahead of the files
//! that wait, one at a time (a newer copy replaces the older), hidden from the panel.
use crate::{
    core::{constant_time_eq, now_ms, Core, Outgoing, Runtime, TransferDirection},
    files::{self, SafeName},
    protocol::MAX_FILE_BYTES_V2,
};
use serde::Serialize;
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::Notify;
use uuid::Uuid;

pub const MAX_OUTGOING: usize = 20;
pub const PUSH_TTL: Duration = Duration::from_secs(30);

pub struct OutItem {
    pub id: Uuid,
    pub path: PathBuf,
    pub name: SafeName,
    pub size: u64,
    /// clipboard.v2: a temporary PNG for the phone's clipboard, deleted when it leaves.
    pub clipboard: Option<ClipPush>,
}

/// The `clipboard` field of a `file.push`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipPush {
    pub revision: u64,
    pub automatic: bool,
}

impl OutItem {
    fn discard(&self) {
        if self.clipboard.is_some() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Wakes the socket task; `by_user` tells it to say "Envio cancelado no PC.".
#[derive(Default)]
pub struct Abort {
    pub notify: Notify,
    pub by_user: AtomicBool,
}

enum Phase {
    Offered { secret: [u8; 32], expires: Instant },
    Sending { confirmed: u64, abort: Arc<Abort> },
}

pub struct OutActive {
    item: OutItem,
    device: Uuid,
    epoch: u64,
    phase: Phase,
}

#[derive(Default)]
pub struct Outbox {
    queue: VecDeque<OutItem>,
    active: Option<OutActive>,
    last: Option<OutgoingResult>,
}

/// The file being offered or sent, for the panel.
#[derive(Clone, Serialize)]
pub struct OutgoingFile {
    pub name: String,
    pub bytes: u64,
    /// Bytes the phone confirmed it wrote.
    pub sent: u64,
    /// Announced, waiting for the phone to start.
    pub waiting: bool,
}

#[derive(Clone, Serialize)]
pub struct OutgoingResult {
    pub at: u64,
    pub ok: bool,
    pub message: String,
}

/// What the file socket needs once the phone redeemed its ticket.
pub struct Fetched {
    pub transfer_id: Uuid,
    pub device: Uuid,
    pub epoch: u64,
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub abort: Arc<Abort>,
    /// A clipboard image: no panel messages.
    pub clipboard: bool,
}

impl Outbox {
    /// The file in flight, unless it is a clipboard image.
    pub fn active(&self) -> Option<OutgoingFile> {
        self.active
            .as_ref()
            .filter(|a| a.item.clipboard.is_none())
            .map(|a| OutgoingFile {
                name: a.item.name.file_name(),
                bytes: a.item.size,
                sent: match a.phase {
                    Phase::Sending { confirmed, .. } => confirmed,
                    Phase::Offered { .. } => 0,
                },
                waiting: matches!(a.phase, Phase::Offered { .. }),
            })
    }
    pub fn queued(&self) -> Vec<String> {
        self.queue
            .iter()
            .filter(|item| item.clipboard.is_none())
            .map(|item| item.name.file_name())
            .collect()
    }
    pub fn last(&self) -> Option<OutgoingResult> {
        self.last.clone()
    }
    fn owns(&self, abort: &Arc<Abort>) -> bool {
        matches!(&self.active, Some(OutActive { phase: Phase::Sending { abort: own, .. }, .. }) if Arc::ptr_eq(own, abort))
    }
}

impl Runtime {
    fn outgoing_result(&mut self, ok: bool, message: String) {
        self.outbox.last = Some(OutgoingResult {
            at: now_ms(),
            ok,
            message,
        });
    }

    /// Ends the active file with `reason`; the queue stays for the next connection.
    /// A clipboard image just goes away.
    pub(crate) fn abort_outgoing(&mut self, reason: &str) {
        let Some(active) = self.outbox.active.take() else {
            return;
        };
        if let Phase::Sending { abort, .. } = &active.phase {
            abort.notify.notify_one();
        }
        if active.item.clipboard.is_some() {
            active.item.discard();
            return;
        }
        let name = active.item.name.file_name();
        self.outgoing_result(false, format!("{name} não foi enviado. {reason}"));
    }

    /// A newer copy or the end of the session: the clipboard image still on its way is
    /// dropped, queued, announced (its ticket stops working) or sending ("unavailable").
    pub(crate) fn drop_clipboard_push(&mut self) {
        self.outbox.queue.retain(|item| {
            if item.clipboard.is_some() {
                item.discard();
            }
            item.clipboard.is_none()
        });
        let clipboard_active = self
            .outbox
            .active
            .as_ref()
            .is_some_and(|a| a.item.clipboard.is_some());
        if clipboard_active {
            if let Some(OutActive {
                phase: Phase::Sending { abort, .. },
                ..
            }) = &self.outbox.active
            {
                abort.by_user.store(true, Ordering::Release);
            }
            self.abort_outgoing("");
        }
    }

    /// Queues a clipboard image ahead of the files that wait, replacing an older one.
    pub(crate) fn enqueue_clipboard(
        &mut self,
        item: OutItem,
        events: &tokio::sync::broadcast::Sender<Outgoing>,
    ) {
        self.drop_clipboard_push();
        self.outbox.queue.push_front(item);
        self.pump_outgoing(events);
    }

    /// Offers the next file when a files.v2 phone is connected and nothing is in flight.
    pub(crate) fn pump_outgoing(&mut self, events: &tokio::sync::broadcast::Sender<Outgoing>) {
        if self.outbox.active.is_some() || !self.files_v2 {
            return;
        }
        let Some(device) = self.connected else {
            return;
        };
        if !self.session_ok(device, self.connection_epoch) {
            return;
        }
        let item = loop {
            let Some(item) = self.outbox.queue.pop_front() else {
                return;
            };
            // Only clipboard.v2 sessions understand the `clipboard` field.
            if item.clipboard.is_some() && !self.clipboard_image {
                item.discard();
                continue;
            }
            break item;
        };
        let mut secret = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::rng(), &mut secret);
        let mut message = serde_json::json!({
            "v": 1,
            "type": "file.push",
            "transferId": item.id,
            "name": item.name.file_name(),
            "size": item.size,
            "mime": files::mime_for(&item.name),
            "ticket": hex::encode(secret),
        });
        if let Some(clip) = item.clipboard {
            message["clipboard"] =
                serde_json::json!({"revision": clip.revision, "automatic": clip.automatic});
        }
        self.outbox.active = Some(OutActive {
            item,
            device,
            epoch: self.connection_epoch,
            phase: Phase::Offered {
                secret,
                expires: Instant::now() + PUSH_TTL,
            },
        });
        let _ = events.send(Outgoing::Wire {
            device_id: device,
            message,
        });
    }
}

/// Size and display name of a file the user picked; refuses folders, empty and oversized files.
fn inspect(path: &std::path::Path) -> Result<(SafeName, u64), String> {
    let raw = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = files::sanitize_name(&raw).fit(255);
    let label = name.file_name();
    let meta = std::fs::metadata(path).map_err(|_| format!("Não foi possível ler {label}."))?;
    if !meta.is_file() {
        return Err(format!("{label} não é um arquivo."));
    }
    match meta.len() {
        0 => Err(format!("{label} está vazio.")),
        size if size > MAX_FILE_BYTES_V2 => Err(format!("{label} passa de 1 GB.")),
        size => Ok((name, size)),
    }
}

impl Core {
    /// Queues files for the phone. Returns one message per file left out.
    pub fn enqueue_outgoing(&self, paths: Vec<PathBuf>) -> Vec<String> {
        let checked: Vec<_> = paths
            .into_iter()
            .map(|path| (inspect(&path), path))
            .collect();
        let mut s = self.state.lock().unwrap();
        let mut problems = Vec::new();
        for (check, path) in checked {
            let (name, size) = match check {
                Ok(found) => found,
                Err(problem) => {
                    problems.push(problem);
                    continue;
                }
            };
            let busy = usize::from(s.outbox.active.is_some());
            if s.outbox.queue.len() + busy >= MAX_OUTGOING {
                problems.push(format!(
                    "A fila aceita até {MAX_OUTGOING} arquivos; {} ficou de fora.",
                    name.file_name()
                ));
                continue;
            }
            if s.outbox.queue.iter().any(|item| item.path == path) {
                continue;
            }
            s.outbox.queue.push_back(OutItem {
                id: Uuid::new_v4(),
                path,
                name,
                size,
                clipboard: None,
            });
        }
        s.pump_outgoing(&self.events);
        problems
    }

    /// Called after the welcome and the first snapshot went out on a new session.
    pub fn outgoing_connected(&self) {
        let mut s = self.state.lock().unwrap();
        s.pump_outgoing(&self.events);
    }

    /// Poll tick: an announced file the phone did not fetch in time gives way to the next.
    pub fn outgoing_tick(&self) {
        let mut s = self.state.lock().unwrap();
        let expired = matches!(&s.outbox.active, Some(OutActive { phase: Phase::Offered { expires, .. }, .. }) if *expires <= Instant::now());
        if expired {
            s.abort_outgoing("O celular não respondeu a tempo.");
        }
        s.pump_outgoing(&self.events);
    }

    /// Redeems the ticket of the announced file. Any attempt naming it consumes it.
    pub fn outgoing_fetch(
        &self,
        transfer_id: Uuid,
        presented: &str,
    ) -> Result<Fetched, &'static str> {
        let mut s = self.state.lock().unwrap();
        let Some(active) = s.outbox.active.as_ref() else {
            return Err("unauthorized");
        };
        let Phase::Offered { secret, expires } = &active.phase else {
            return Err("unauthorized");
        };
        if active.item.id != transfer_id {
            return Err("unauthorized");
        }
        let presented = files::is_sha256_hex(presented)
            .then(|| hex::decode(presented).ok())
            .flatten()
            .unwrap_or_default();
        let valid = constant_time_eq(&presented, secret)
            && *expires > Instant::now()
            && s.session_ok(active.device, active.epoch)
            && s.files_v2;
        if !valid {
            s.abort_outgoing("O celular não foi autorizado a receber.");
            s.pump_outgoing(&self.events);
            return Err("unauthorized");
        }
        let abort = Arc::new(Abort::default());
        let active = s.outbox.active.as_mut().unwrap();
        active.phase = Phase::Sending {
            confirmed: 0,
            abort: abort.clone(),
        };
        Ok(Fetched {
            transfer_id,
            device: active.device,
            epoch: active.epoch,
            path: active.item.path.clone(),
            name: active.item.name.file_name(),
            size: active.item.size,
            abort,
            clipboard: active.item.clipboard.is_some(),
        })
    }

    /// Per-message check of the sending socket; also records what the phone confirmed.
    pub fn outgoing_check(&self, fetched: &Fetched, confirmed: u64) -> Result<(), &'static str> {
        let mut s = self.state.lock().unwrap();
        if !s.session_ok(fetched.device, fetched.epoch) {
            return Err("unauthorized");
        }
        match &mut s.outbox.active {
            Some(OutActive {
                phase:
                    Phase::Sending {
                        confirmed: own,
                        abort,
                    },
                ..
            }) if Arc::ptr_eq(abort, &fetched.abort) => {
                *own = confirmed;
                Ok(())
            }
            _ => Err("cancelled"),
        }
    }

    /// The socket task finished: `Ok(final name on the phone)` or `Err(reason for the panel)`.
    pub fn outgoing_finish(&self, fetched: &Fetched, outcome: Result<String, String>) {
        let mut s = self.state.lock().unwrap();
        if s.outbox.owns(&fetched.abort) {
            let active = s.outbox.active.take().unwrap();
            if let Some(clip) = active.item.clipboard {
                active.item.discard();
                if outcome.is_ok() {
                    s.record_image(TransferDirection::ToPhone, active.item.size, clip.automatic);
                }
                s.pump_outgoing(&self.events);
                return;
            }
            let name = active.item.name.file_name();
            match outcome {
                Ok(_) => {
                    s.record_file(&name, active.item.size, TransferDirection::ToPhone);
                    s.outgoing_result(true, format!("{name} enviado ao celular."));
                }
                Err(reason) => {
                    s.outgoing_result(false, format!("{name} não foi enviado. {reason}"))
                }
            }
        }
        s.pump_outgoing(&self.events);
    }

    /// `file.cancel` or `file.decline` from the phone for the file it was offered or is receiving.
    pub fn outgoing_refused(&self, device: Uuid, epoch: u64, transfer_id: Uuid, reason: &str) {
        let mut s = self.state.lock().unwrap();
        let owned =
            s.outbox.active.as_ref().is_some_and(|a| {
                a.item.id == transfer_id && a.device == device && a.epoch == epoch
            });
        if owned {
            s.abort_outgoing(reason);
            s.pump_outgoing(&self.events);
        }
    }

    /// "Cancelar" in the panel; a clipboard image is not shown there and is left alone.
    pub fn cancel_outgoing(&self) {
        let mut s = self.state.lock().unwrap();
        if s.outbox
            .active
            .as_ref()
            .is_some_and(|a| a.item.clipboard.is_some())
        {
            return;
        }
        if let Some(OutActive {
            phase: Phase::Sending { abort, .. },
            ..
        }) = &s.outbox.active
        {
            abort.by_user.store(true, Ordering::Release);
        }
        if let Some(active) = &s.outbox.active {
            let name = active.item.name.file_name();
            s.abort_outgoing("");
            s.outgoing_result(false, format!("Envio de {name} cancelado."));
        }
        s.pump_outgoing(&self.events);
    }

    pub fn clear_outgoing(&self) {
        let mut s = self.state.lock().unwrap();
        s.outbox.queue.retain(|item| item.clipboard.is_some());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        protocol::Features,
        secrets::{self, Device, Secrets},
    };
    use std::fs;

    fn session(files_v2: bool) -> (Arc<Core>, Uuid, String, u64) {
        let core = Core::ephemeral(
            Secrets {
                pc_id: Uuid::new_v4(),
                certificate_pem: String::new(),
                private_key_pem: String::new(),
                devices: Vec::new(),
            },
            "00".into(),
        );
        let id = Uuid::new_v4();
        let token = crate::core::random_hex();
        core.state.lock().unwrap().secrets.devices.push(Device {
            id,
            name: "Outbox test".into(),
            token_hash: secrets::token_hash(&token),
        });
        let features = Features {
            files: true,
            files_v2,
            ..Features::default()
        };
        let epoch = core.connect_authenticated(id, &token, features).unwrap();
        (core, id, token, epoch)
    }

    fn sample(label: &str, bytes: &[u8]) -> PathBuf {
        let folder = files::tests::temp_folder(label);
        let path = folder.join(format!("{label}.bin"));
        fs::write(&path, bytes).unwrap();
        path
    }

    fn pushed(events: &mut tokio::sync::broadcast::Receiver<Outgoing>) -> serde_json::Value {
        loop {
            match events.try_recv().expect("file.push") {
                Outgoing::Wire { message, .. } if message["type"] == "file.push" => return message,
                _ => {}
            }
        }
    }

    #[test]
    fn rejects_folders_empty_and_oversized_files_and_bounds_the_queue() {
        let (core, ..) = session(false);
        let folder = files::tests::temp_folder("outbox-reject");
        let empty = folder.join("vazio.txt");
        fs::write(&empty, b"").unwrap();
        let problems = core.enqueue_outgoing(vec![folder.clone(), empty, folder.join("nada.txt")]);
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems[1].contains("vazio"));
        let paths: Vec<_> = (0..MAX_OUTGOING + 2)
            .map(|n| {
                let path = folder.join(format!("a{n}.txt"));
                fs::write(&path, b"x").unwrap();
                path
            })
            .collect();
        let problems = core.enqueue_outgoing(paths.clone());
        assert_eq!(problems.len(), 2);
        // A phone without files.v2 gets nothing: the queue waits.
        assert!(core.status().outgoing.is_none());
        assert_eq!(core.status().outgoing_queue.len(), MAX_OUTGOING);
        assert!(core.enqueue_outgoing(vec![paths[0].clone()]).len() <= 1);
        core.clear_outgoing();
        assert!(core.status().outgoing_queue.is_empty());
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn push_ticket_is_single_use_epoch_bound_and_expires() {
        let (core, id, token, epoch) = session(true);
        let mut events = core.events.subscribe();
        let path = sample("outbox-ticket", b"conteudo");
        assert!(core
            .enqueue_outgoing(vec![path.clone(), path.clone()])
            .is_empty());
        let push = pushed(&mut events);
        assert_eq!(push["size"], 8);
        assert_eq!(push["mime"], "application/octet-stream");
        let transfer: Uuid = push["transferId"].as_str().unwrap().parse().unwrap();
        let ticket = push["ticket"].as_str().unwrap().to_string();
        assert!(core.status().outgoing.unwrap().waiting);
        // The duplicate path was not queued twice.
        assert!(core.status().outgoing_queue.is_empty());
        assert_eq!(
            core.outgoing_fetch(Uuid::new_v4(), &ticket).err(),
            Some("unauthorized")
        );
        let fetched = core.outgoing_fetch(transfer, &ticket).unwrap();
        assert_eq!(fetched.size, 8);
        assert_eq!(
            core.outgoing_fetch(transfer, &ticket).err(),
            Some("unauthorized")
        );
        core.outgoing_check(&fetched, 4).unwrap();
        assert_eq!(core.status().outgoing.unwrap().sent, 4);
        core.outgoing_finish(&fetched, Ok("outbox-ticket.bin".into()));
        let status = core.status();
        assert!(status.outgoing.is_none());
        assert!(status.outgoing_result.unwrap().ok);
        assert_eq!(status.file_history[0].direction, TransferDirection::ToPhone);

        // A wrong secret burns the offer.
        core.enqueue_outgoing(vec![path.clone()]);
        let push = pushed(&mut events);
        let transfer: Uuid = push["transferId"].as_str().unwrap().parse().unwrap();
        assert_eq!(
            core.outgoing_fetch(transfer, &"0".repeat(64)).err(),
            Some("unauthorized")
        );
        assert!(core.status().outgoing.is_none());
        assert!(!core.status().outgoing_result.unwrap().ok);

        // Expired offers give way; a new epoch invalidates the offer of the old one.
        core.enqueue_outgoing(vec![path.clone()]);
        let _ = pushed(&mut events);
        if let Some(OutActive {
            phase: Phase::Offered { expires, .. },
            ..
        }) = &mut core.state.lock().unwrap().outbox.active
        {
            *expires = Instant::now() - Duration::from_millis(1);
        }
        core.outgoing_tick();
        assert!(core.status().outgoing.is_none());
        core.enqueue_outgoing(vec![path.clone()]);
        let push = pushed(&mut events);
        let transfer: Uuid = push["transferId"].as_str().unwrap().parse().unwrap();
        let ticket = push["ticket"].as_str().unwrap().to_string();
        let features = Features {
            files: true,
            files_v2: true,
            ..Features::default()
        };
        let fresh = core.connect_authenticated(id, &token, features).unwrap();
        assert_ne!(fresh, epoch);
        assert_eq!(
            core.outgoing_fetch(transfer, &ticket).err(),
            Some("unauthorized")
        );
        // The queue waits for the next connection instead of being dropped.
        core.enqueue_outgoing(vec![path.clone()]);
        assert!(core.status().outgoing_queue.len() <= 1);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn cancel_from_the_panel_or_the_phone_stops_the_socket_and_moves_on() {
        let (core, id, _, epoch) = session(true);
        let mut events = core.events.subscribe();
        let first = sample("outbox-cancel-a", b"aaaa");
        let second = sample("outbox-cancel-b", b"bbbb");
        core.enqueue_outgoing(vec![first.clone(), second.clone()]);
        let push = pushed(&mut events);
        let transfer: Uuid = push["transferId"].as_str().unwrap().parse().unwrap();
        let fetched = core
            .outgoing_fetch(transfer, push["ticket"].as_str().unwrap())
            .unwrap();
        core.cancel_outgoing();
        assert!(fetched.abort.by_user.load(Ordering::Acquire));
        assert_eq!(core.outgoing_check(&fetched, 1), Err("cancelled"));
        assert!(core
            .status()
            .outgoing_result
            .unwrap()
            .message
            .contains("cancelado"));
        // The late finish of the cancelled socket does not touch the next file.
        let next = pushed(&mut events);
        core.outgoing_finish(&fetched, Err("x".into()));
        assert!(core.status().outgoing.unwrap().waiting);
        let transfer: Uuid = next["transferId"].as_str().unwrap().parse().unwrap();
        core.outgoing_refused(id, epoch, Uuid::new_v4(), "outro");
        assert!(core.status().outgoing.is_some());
        core.outgoing_refused(id, epoch, transfer, "Sem espaço no celular.");
        let status = core.status();
        assert!(status.outgoing.is_none());
        assert!(status
            .outgoing_result
            .unwrap()
            .message
            .contains("Sem espaço"));
        for path in [first, second] {
            fs::remove_dir_all(path.parent().unwrap()).unwrap();
        }
    }
}
