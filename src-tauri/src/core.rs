pub use crate::pairing::{Pairing, Pending, PendingView};
use crate::{
    audio,
    clip_images::ImageWriter,
    clipboard,
    energy::{PowerScheduleView, PowerSystem, Scheduled},
    files::{FileEntry, IncomingFile, Slot},
    media,
    outbox::{Outbox, OutgoingFile, OutgoingResult},
    protocol::{
        self, Action, AudioState, Features, FilesState, InputState, MediaState, StateSnapshot,
        SyncSettings,
    },
    secrets::{self, Secrets},
    settings::{self, Settings},
};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Clone, Serialize)]
pub struct DeviceView {
    pub id: Uuid,
    pub name: String,
    pub connected: bool,
    /// Unix milliseconds of the latest connect or disconnect seen by this
    /// process. Kept in RAM only, so it is `None` after the app restarts.
    #[serde(rename = "lastSeenAt")]
    pub last_seen_at: Option<u64>,
}
#[derive(Clone, Serialize)]
pub struct DiagnosticEntry {
    pub at: u64,
    pub message: String,
}
#[derive(Clone, Copy, Serialize, PartialEq, Eq, Debug)]
pub enum TransferDirection {
    #[serde(rename = "toPhone")]
    ToPhone,
    #[serde(rename = "toPc")]
    ToPc,
}
#[derive(Clone, Copy, Serialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum TransferKind {
    Text,
    /// clipboard.v2.
    Image,
}
/// Clipboard transfer metadata for the desktop panel. The copied text or image itself is
/// never retained: only direction, time, size and whether it was automatic.
#[derive(Clone, Serialize)]
pub struct TransferEntry {
    pub at: u64,
    pub direction: TransferDirection,
    pub kind: TransferKind,
    /// Text only; 0 for images.
    pub characters: usize,
    /// Images only; 0 for text.
    pub bytes: u64,
    pub automatic: bool,
}
const MAX_DIAGNOSTICS: usize = 20;
const MAX_TRANSFERS: usize = 12;
#[derive(Clone, Serialize)]
pub struct Status {
    #[serde(rename = "serviceRunning")]
    pub service_running: bool,
    #[serde(rename = "pcId")]
    pub pc_id: Uuid,
    #[serde(rename = "pcName")]
    pub pc_name: String,
    pub addresses: Vec<String>,
    pub port: u16,
    pub fingerprint: String,
    pub pairing: Option<Pairing>,
    pub devices: Vec<DeviceView>,
    pub pending: Vec<PendingView>,
    pub settings: Settings,
    pub diagnostics: Vec<DiagnosticEntry>,
    pub transfers: Vec<TransferEntry>,
    #[serde(rename = "inputActive")]
    pub input_active: bool,
    /// Files in both directions, newest first; names and sizes only.
    #[serde(rename = "fileHistory")]
    pub file_history: Vec<FileEntry>,
    #[serde(rename = "incomingFile")]
    pub incoming_file: Option<IncomingFile>,
    /// The file being offered to or sent to the phone.
    pub outgoing: Option<OutgoingFile>,
    /// Names waiting to be sent, in order.
    #[serde(rename = "outgoingQueue")]
    pub outgoing_queue: Vec<String>,
    /// Outcome of the last file sent to the phone.
    #[serde(rename = "outgoingResult")]
    pub outgoing_result: Option<OutgoingResult>,
    /// The connected phone negotiated files.v2 and accepts files from the PC.
    #[serde(rename = "phoneReceivesFiles")]
    pub phone_receives_files: bool,
    /// power.v1: the action a phone scheduled on this PC.
    #[serde(rename = "powerScheduled")]
    pub power_scheduled: Option<PowerScheduleView>,
}
#[derive(Clone)]
pub enum Outgoing {
    Wire { device_id: Uuid, message: Value },
    Close { device_id: Uuid },
}
pub struct Runtime {
    pub service_running: bool,
    pub secrets: Secrets,
    pub settings: Settings,
    pub pc_name: String,
    pub addresses: Vec<String>,
    pub fingerprint: String,
    pub pairing: Option<(Pairing, Instant)>,
    pub pair_generation: u64,
    pub pending: HashMap<Uuid, Pending>,
    pub connected: Option<Uuid>,
    pub connection_epoch: u64,
    pub failed_pairs: VecDeque<Instant>,
    pub revoked_devices: HashSet<Uuid>,
    pub audio: AudioState,
    pub media: MediaState,
    pub revision: u64,
    pub clipboard_revision: u64,
    pub clipboard_sequence: u32,
    pub suppressed_sequence: u32,
    pub event_ids: VecDeque<Uuid>,
    pub event_set: HashSet<Uuid>,
    pub client_sync: SyncSettings,
    pub input_feature: bool,
    pub input_enabled: bool,
    pub input_events: VecDeque<Instant>,
    pub files_feature: bool,
    pub files_v2: bool,
    /// clipboard.v2 negotiated on this session (with files.v2).
    pub clipboard_image: bool,
    /// power.v1 negotiated on this session.
    pub power_feature: bool,
    /// One schedule per PC, whatever phone made it; RAM only.
    pub power_schedule: Option<Scheduled>,
    pub power_events: VecDeque<Instant>,
    pub file_slot: Option<Slot>,
    pub outbox: Outbox,
    pub file_history: VecDeque<FileEntry>,
    pub search_events: VecDeque<Instant>,
    pub diagnostics: VecDeque<DiagnosticEntry>,
    pub transfers: VecDeque<TransferEntry>,
    pub last_seen: HashMap<Uuid, u64>,
}
impl Runtime {
    pub(crate) fn session_ok(&self, id: Uuid, epoch: u64) -> bool {
        self.connected == Some(id)
            && self.connection_epoch == epoch
            && !self.revoked_devices.contains(&id)
            && self.secrets.devices.iter().any(|d| d.id == id)
    }
    /// Records the moment a device stopped or started being the active session.
    fn mark_seen(&mut self, id: Uuid) {
        self.last_seen.insert(id, now_ms());
    }
    fn record_transfer(&mut self, direction: TransferDirection, text: &str, automatic: bool) {
        self.push_transfer(TransferEntry {
            at: now_ms(),
            direction,
            kind: TransferKind::Text,
            characters: text.chars().count(),
            bytes: 0,
            automatic,
        });
    }
    pub(crate) fn record_image(
        &mut self,
        direction: TransferDirection,
        bytes: u64,
        automatic: bool,
    ) {
        self.push_transfer(TransferEntry {
            at: now_ms(),
            direction,
            kind: TransferKind::Image,
            characters: 0,
            bytes,
            automatic,
        });
    }
    fn push_transfer(&mut self, entry: TransferEntry) {
        if self.transfers.len() == MAX_TRANSFERS {
            self.transfers.pop_front();
        }
        self.transfers.push_back(entry);
    }
    /// What a session negotiated, cleared when it ends.
    fn clear_session_features(&mut self) {
        self.client_sync = SyncSettings::default();
        self.input_feature = false;
        self.input_enabled = false;
        self.input_events.clear();
        self.power_feature = false;
        self.clipboard_image = false;
    }
}
/// Shows a native notification (title, body). Injected so tests never show toasts.
pub type Notifier = Box<dyn Fn(&str, &str) + Send + Sync>;

pub struct Core {
    last_live_tick: AtomicU64,
    pub state: Mutex<Runtime>,
    pub events: broadcast::Sender<Outgoing>,
    pub(crate) persist_secrets: bool,
    /// Parent of the `PC Remote` folder; `None` resolves the Downloads known folder.
    pub(crate) files_root: Option<PathBuf>,
    pub(crate) files_cleaned: AtomicBool,
    pub(crate) notifier: OnceLock<Notifier>,
    /// power.v1 actions; only the app installs the real one (lib.rs).
    pub(crate) power_system: OnceLock<Box<dyn PowerSystem>>,
    /// clipboard.v2 writes of images from the phone; only the app installs the real one.
    pub(crate) image_writer: OnceLock<ImageWriter>,
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub(crate) fn random_hex() -> String {
    use rand::RngCore;
    let mut b = [0u8; 32];
    rand::rng().fill_bytes(&mut b);
    hex::encode(b)
}
pub(crate) fn addresses() -> Vec<String> {
    let candidates = local_ip_address::list_afinet_netifas()
        .unwrap_or_default()
        .into_iter()
        .map(|(_, ip)| ip)
        .collect();
    // On Windows local_ip() uses the IP Helper default-route table. Put that
    // interface first so the QR code does not advertise a virtual adapter.
    ordered_addresses(candidates, local_ip_address::local_ip().ok())
}

fn ordered_addresses(
    candidates: Vec<std::net::IpAddr>,
    preferred: Option<std::net::IpAddr>,
) -> Vec<String> {
    let mut a = candidates
        .into_iter()
        .filter_map(|ip| match ip {
            std::net::IpAddr::V4(v4)
                if !v4.is_loopback() && !v4.is_link_local() && !v4.is_unspecified() =>
            {
                Some(v4.to_string())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    a.sort();
    a.dedup();
    if let Some(preferred) = preferred.map(|ip| ip.to_string()) {
        if let Some(position) = a.iter().position(|ip| ip == &preferred) {
            a.remove(position);
            a.insert(0, preferred);
        }
    }
    a
}

impl Core {
    #[cfg(test)]
    pub(crate) fn ephemeral(secrets: Secrets, fingerprint: String) -> Arc<Self> {
        // Tests never write to the real Downloads folder.
        let root = std::env::temp_dir().join(format!("pc-remote-test-{}", Uuid::new_v4()));
        let core = Self::build(secrets, fingerprint, false, Some(root));
        core.state.lock().unwrap().settings = Settings::default();
        core
    }
    pub fn new(secrets: Secrets, fingerprint: String) -> Arc<Self> {
        Self::build(secrets, fingerprint, true, None)
    }
    fn build(
        secrets: Secrets,
        fingerprint: String,
        persist_secrets: bool,
        files_root: Option<PathBuf>,
    ) -> Arc<Self> {
        let (events, _) = broadcast::channel(64);
        let name = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC Windows".into());
        Arc::new(Self {
            last_live_tick: AtomicU64::new(unsafe {
                windows::Win32::System::SystemInformation::GetTickCount64()
            }),
            state: Mutex::new(Runtime {
                service_running: false,
                secrets,
                settings: settings::load(),
                pc_name: name,
                addresses: addresses(),
                fingerprint,
                pairing: None,
                pair_generation: 0,
                pending: HashMap::new(),
                connected: None,
                connection_epoch: 0,
                failed_pairs: VecDeque::new(),
                revoked_devices: HashSet::new(),
                audio: audio::read(),
                media: media::read(),
                revision: 0,
                clipboard_revision: 0,
                clipboard_sequence: clipboard::sequence(),
                suppressed_sequence: 0,
                event_ids: VecDeque::new(),
                event_set: HashSet::new(),
                client_sync: SyncSettings::default(),
                input_feature: false,
                input_enabled: false,
                input_events: VecDeque::new(),
                files_feature: false,
                files_v2: false,
                clipboard_image: false,
                power_feature: false,
                power_schedule: None,
                power_events: VecDeque::new(),
                file_slot: None,
                outbox: Outbox::default(),
                file_history: VecDeque::new(),
                search_events: VecDeque::new(),
                diagnostics: VecDeque::new(),
                transfers: VecDeque::new(),
                last_seen: HashMap::new(),
            }),
            events,
            persist_secrets,
            files_root,
            files_cleaned: AtomicBool::new(false),
            notifier: OnceLock::new(),
            power_system: OnceLock::new(),
            image_writer: OnceLock::new(),
        })
    }
    pub fn set_notifier(&self, notifier: Notifier) {
        let _ = self.notifier.set(notifier);
    }
    pub fn status(&self) -> Status {
        let s = self.state.lock().unwrap();
        let pairing = s.pairing.as_ref().and_then(|(p, deadline)| {
            if *deadline > Instant::now() {
                Some(p.clone())
            } else {
                None
            }
        });
        Status {
            service_running: s.service_running,
            pc_id: s.secrets.pc_id,
            pc_name: s.pc_name.clone(),
            addresses: addresses(),
            port: protocol::PORT,
            fingerprint: s.fingerprint.clone(),
            pairing,
            devices: s
                .secrets
                .devices
                .iter()
                .map(|d| DeviceView {
                    id: d.id,
                    name: d.name.clone(),
                    connected: s.connected == Some(d.id),
                    last_seen_at: s.last_seen.get(&d.id).copied(),
                })
                .collect(),
            pending: s
                .pending
                .iter()
                .map(|(id, p)| PendingView {
                    id: *id,
                    device_name: p.device_name.clone(),
                    requested_at: p.requested_at,
                })
                .collect(),
            settings: s.settings.clone(),
            diagnostics: s.diagnostics.iter().cloned().collect(),
            transfers: s.transfers.iter().rev().cloned().collect(),
            input_active: s.input_enabled,
            file_history: s.file_history.iter().rev().cloned().collect(),
            incoming_file: s.incoming_file(),
            outgoing: s.outbox.active(),
            outgoing_queue: s.outbox.queued(),
            outgoing_result: s.outbox.last(),
            phone_receives_files: s.connected.is_some() && s.files_v2,
            power_scheduled: s.power_view(),
        }
    }
    pub fn connect_authenticated(
        &self,
        id: Uuid,
        token: &str,
        features: Features,
    ) -> Result<u64, &'static str> {
        if id.is_nil() || token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("unauthorized");
        }
        let hash = Sha256::digest(token.as_bytes());
        let mut s = self.state.lock().unwrap();
        if s.revoked_devices.contains(&id)
            || !s.secrets.devices.iter().any(|d| {
                d.id == id
                    && hex::decode(&d.token_hash)
                        .ok()
                        .is_some_and(|h| constant_time_eq(&h, &hash))
            })
        {
            return Err("unauthorized");
        }
        if s.connected.is_some_and(|current| current != id) {
            return Err("unavailable");
        }
        if s.connected == Some(id) {
            let _ = self.events.send(Outgoing::Close { device_id: id });
        }
        s.connected = Some(id);
        s.mark_seen(id);
        s.connection_epoch = s.connection_epoch.wrapping_add(1);
        s.clear_session_features();
        s.input_feature = features.input;
        s.abort_file();
        s.drop_clipboard_push();
        s.abort_outgoing("A conexão com o celular foi reiniciada.");
        s.files_feature = features.files;
        s.files_v2 = features.files_v2;
        s.clipboard_image = features.clipboard_image && features.files_v2;
        s.power_feature = features.power;
        Ok(s.connection_epoch)
    }
    pub fn is_authorized_session(&self, id: Uuid, epoch: u64) -> bool {
        self.state.lock().unwrap().session_ok(id, epoch)
    }
    #[cfg(test)]
    pub fn is_authorized_connection(&self, id: Uuid) -> bool {
        let s = self.state.lock().unwrap();
        s.connected == Some(id)
            && !s.revoked_devices.contains(&id)
            && s.secrets.devices.iter().any(|d| d.id == id)
    }
    pub fn disconnect(&self, id: Uuid, epoch: u64) {
        let mut s = self.state.lock().unwrap();
        if s.connected == Some(id) && s.connection_epoch == epoch {
            s.connected = None;
            s.mark_seen(id);
            s.clear_session_features();
            s.end_file_session();
        }
    }
    pub fn revoke(&self, id: Uuid) -> Result<(), String> {
        self.revoke_with_save(id, |candidate| {
            if self.persist_secrets {
                secrets::save(candidate)?;
            }
            Ok(())
        })
    }
    pub(crate) fn revoke_with_save(
        &self,
        id: Uuid,
        save: impl FnOnce(&Secrets) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut s = self.state.lock().unwrap();
        if !s.secrets.devices.iter().any(|d| d.id == id) {
            return Err("Dispositivo não encontrado".into());
        }
        // Deny reconnection in this process even if durable revocation fails.
        // Keep the credential list consistent with disk for a deletion retry.
        s.revoked_devices.insert(id);
        s.last_seen.remove(&id);
        // A revoked phone's schedule goes with it; another connected phone hears of it.
        if s.power_schedule.as_ref().is_some_and(|p| p.device == id) {
            s.drop_power_schedule();
            if s.connected.is_some_and(|connected| connected != id) {
                self.publish_state_locked(&s);
            }
        }
        if s.connected == Some(id) {
            s.connected = None;
            s.connection_epoch = s.connection_epoch.wrapping_add(1);
            s.clear_session_features();
            s.end_file_session();
            s.revision += 1;
            let _ = self.events.send(Outgoing::Close { device_id: id });
        }
        let mut candidate = s.secrets.clone();
        candidate.devices.retain(|d| d.id != id);
        save(&candidate).map_err(|error| format!(
            "Não foi possível salvar a revogação: {error}. Acesso bloqueado nesta execução; tente revogar novamente. Reiniciar antes de concluir pode restaurar o acesso."
        ))?;
        s.secrets = candidate;
        Ok(())
    }
    pub fn update_settings(&self, new: Settings) -> Result<(), String> {
        self.update_settings_with_io(new, settings::set_autostart, settings::save)
    }
    pub(crate) fn update_settings_with_io(
        &self,
        new: Settings,
        mut set_autostart: impl FnMut(bool) -> Result<(), String>,
        save: impl FnOnce(&Settings) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut s = self.state.lock().unwrap();
        // Only "Interromper" and "Liberar" move the block (see set_input_blocked_with_save).
        let mut new = new;
        new.input_blocked = s.settings.input_blocked;
        let autostart_changed = new.autostart != s.settings.autostart;
        if autostart_changed {
            set_autostart(new.autostart)?;
        }
        if let Err(error) = save(&new) {
            if autostart_changed {
                if let Err(rollback) = set_autostart(s.settings.autostart) {
                    return Err(format!(
                        "{error}; falha ao restaurar inicialização: {rollback}"
                    ));
                }
            }
            return Err(error);
        }
        s.settings = new;
        s.revision += 1;
        self.publish_state_locked(&s);
        Ok(())
    }
    /// "Liberar" in the panel ends a block set by "Interromper".
    pub fn unblock_input(&self) -> Result<(), String> {
        self.set_input_blocked_with_save(false, settings::save)
    }
    /// "Interromper" and "Liberar" are the only writers of `input_blocked`, read and
    /// written under one lock, so a panel holding older settings cannot undo them.
    pub(crate) fn set_input_blocked_with_save(
        &self,
        blocked: bool,
        save: impl FnOnce(&Settings) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut s = self.state.lock().unwrap();
        let mut next = s.settings.clone();
        next.input_blocked = blocked;
        let saved = save(&next);
        if saved.is_err() && !blocked {
            // Lifting a block that could not be saved would come back after a restart.
            return saved.map_err(|error| format!("Não foi possível liberar: {error}"));
        }
        if blocked {
            s.input_enabled = false;
            s.input_events.clear();
            // "Interromper" also blocks power actions and drops what was scheduled.
            s.power_schedule = None;
        }
        s.settings = next;
        s.revision += 1;
        self.publish_state_locked(&s);
        saved.map_err(|error| {
            format!(
                "Mouse e teclado bloqueados nesta execução, mas não foi possível salvar: {error}"
            )
        })
    }
    pub fn snapshot(&self) -> StateSnapshot {
        let s = self.state.lock().unwrap();
        Self::snapshot_locked(&s)
    }
    fn snapshot_locked(s: &Runtime) -> StateSnapshot {
        StateSnapshot {
            v: 1,
            kind: "state",
            revision: s.revision,
            audio: s.audio.clone(),
            media: s.media.clone(),
            clipboard_revision: s.clipboard_revision,
            sync: s.client_sync.clone(),
            input: s.input_feature.then_some(InputState {
                allowed: !s.settings.input_blocked,
                enabled: s.input_enabled,
            }),
            // Receiving needs no switch since 0.6; files.v2 sessions get the larger limit.
            files: s.files_feature.then_some(FilesState {
                allowed: true,
                max_bytes: if s.files_v2 {
                    protocol::MAX_FILE_BYTES_V2
                } else {
                    protocol::MAX_FILE_BYTES
                },
            }),
            power: s.power_state(),
        }
    }
    #[cfg(test)]
    pub fn input_active(&self, id: Uuid, epoch: u64) -> bool {
        let s = self.state.lock().unwrap();
        s.connected == Some(id)
            && s.connection_epoch == epoch
            && !s.revoked_devices.contains(&id)
            && s.input_feature
            && s.input_enabled
            && !s.settings.input_blocked
    }
    pub fn input_enable(&self, id: Uuid, epoch: u64, enabled: bool) -> Result<(), String> {
        let mut s = self.state.lock().unwrap();
        if s.connected != Some(id)
            || s.connection_epoch != epoch
            || s.revoked_devices.contains(&id)
            || !s.secrets.devices.iter().any(|d| d.id == id)
        {
            return Err("unauthorized".into());
        }
        if !s.input_feature || (enabled && s.settings.input_blocked) {
            return Err("unavailable".into());
        }
        if s.input_enabled != enabled {
            s.input_enabled = enabled;
            s.input_events.clear();
            s.revision += 1;
            self.publish_state_locked(&s);
        }
        Ok(())
    }
    /// "Interromper": disarms at once and blocks mouse and keyboard until "Liberar".
    /// The block is saved, so neither a queued enable nor a reconnection undoes it;
    /// the connection itself stays for media, clipboard and files.
    pub fn stop_input(&self) -> Result<(), String> {
        self.stop_input_with_save(settings::save)
    }
    pub(crate) fn stop_input_with_save(
        &self,
        save: impl FnOnce(&Settings) -> Result<(), String>,
    ) -> Result<(), String> {
        // Blocked in RAM even if the disk write fails, so this run stays safe.
        self.set_input_blocked_with_save(true, save)
    }
    pub fn execute_input(&self, id: Uuid, epoch: u64, action: &Action) -> Result<(), String> {
        let mut s = self.state.lock().unwrap();
        if s.connected != Some(id)
            || s.connection_epoch != epoch
            || s.revoked_devices.contains(&id)
            || !s.secrets.devices.iter().any(|d| d.id == id)
        {
            return Err("unauthorized".into());
        }
        if !s.input_feature || !s.input_enabled || s.settings.input_blocked {
            return Err("unavailable".into());
        }
        let now = Instant::now();
        while s
            .input_events
            .front()
            .is_some_and(|old| now.duration_since(*old).as_secs_f64() >= 1.0)
        {
            s.input_events.pop_front();
        }
        if s.input_events.len() >= 120 {
            return Err("rate_limited".into());
        }
        s.input_events.push_back(now);
        crate::input::execute(action).map_err(str::to_string)
    }
    pub(crate) fn publish_state_locked(&self, s: &Runtime) {
        if let Some(id) = s.connected {
            let _ = self.events.send(Outgoing::Wire {
                device_id: id,
                message: serde_json::to_value(Self::snapshot_locked(s)).unwrap(),
            });
        }
    }
    /// Poll tick. A moving playback position alone publishes nothing: no screen shows it,
    /// and a snapshot every tick while something plays would only cost battery.
    pub fn refresh(&self) {
        let expected_revision = self.state.lock().unwrap().revision;
        let audio = audio::read();
        let media = media::read();
        let mut s = self.state.lock().unwrap();
        if s.revision != expected_revision {
            return;
        }
        let before = serde_json::to_string(&(&s.audio, media_key(&s.media))).unwrap_or_default();
        let after = serde_json::to_string(&(&audio, media_key(&media))).unwrap_or_default();
        if before != after {
            s.audio = audio;
            s.media = media;
            s.revision += 1;
            self.publish_state_locked(&s);
        }
    }
    /// Re-reads the sessions after a command; players apply it asynchronously.
    pub fn media_followup(&self) {
        let media = media::read();
        let mut s = self.state.lock().unwrap();
        if serde_json::to_string(&media_key(&s.media)).ok()
            != serde_json::to_string(&media_key(&media)).ok()
        {
            s.media = media;
            s.revision += 1;
            self.publish_state_locked(&s);
        }
    }
    /// search.v1 limit: at most 5 searches in 10 s.
    pub fn search_allowed(&self) -> bool {
        let mut s = self.state.lock().unwrap();
        let now = Instant::now();
        while s
            .search_events
            .front()
            .is_some_and(|at| now.duration_since(*at).as_secs() >= 10)
        {
            s.search_events.pop_front();
        }
        if s.search_events.len() >= 5 {
            return false;
        }
        s.search_events.push_back(now);
        true
    }
    pub fn set_client_sync(&self, upload: bool, download: bool, paused: bool) {
        let mut s = self.state.lock().unwrap();
        s.client_sync = SyncSettings {
            upload,
            download,
            paused,
        };
        s.revision += 1;
        self.publish_state_locked(&s);
    }
    /// A copy in Windows. Text goes in a `clipboard.event`; without text, a clipboard.v2
    /// session gets the image as a push (read and converted after the lock is released).
    pub fn clipboard_event_local(&self, sequence: u32) {
        let image = {
            let mut s = self.state.lock().unwrap();
            if sequence == s.clipboard_sequence || sequence == s.suppressed_sequence {
                return;
            }
            if clipboard::sequence() != sequence {
                return;
            }
            // Any newer copy replaces an image still on its way to the phone.
            s.drop_clipboard_push();
            let read = s.download_permitted().then(clipboard::read);
            if clipboard::sequence() != sequence {
                return;
            }
            s.clipboard_sequence = sequence;
            s.clipboard_revision += 1;
            s.revision += 1;
            self.publish_state_locked(&s);
            match read {
                Some(Ok(Some(text))) => {
                    self.send_text_event(&mut s, text);
                    None
                }
                Some(Ok(None)) if s.clipboard_image => s
                    .connected
                    .map(|device| (device, s.connection_epoch, s.clipboard_revision)),
                _ => None,
            }
        };
        if let Some((device, epoch, revision)) = image {
            self.clipboard_image_local(sequence, device, epoch, revision);
        }
    }
    fn send_text_event(&self, s: &mut Runtime, text: String) {
        if let Some(device_id) = s.connected {
            s.record_transfer(TransferDirection::ToPhone, &text, true);
            let event = protocol::ClipboardEvent {
                v: 1,
                kind: "clipboard.event",
                event_id: Uuid::new_v4(),
                origin: "windows",
                revision: s.clipboard_revision,
                text,
                automatic: true,
            };
            let _ = self.events.send(Outgoing::Wire {
                device_id,
                message: serde_json::to_value(event).unwrap(),
            });
        }
    }
    pub fn clipboard_get(&self, device: Uuid) -> Result<(), String> {
        let mut s = self.state.lock().unwrap();
        let before = clipboard::sequence();
        let text = clipboard::read()?;
        if clipboard::sequence() != before {
            return Err("clipboard_busy".into());
        }
        let Some(text) = text else {
            // clipboard.v2: no text, so the image, if there is one, goes as a push.
            if !s.clipboard_image {
                return Err("unavailable".into());
            }
            let (epoch, revision) = (s.connection_epoch, s.clipboard_revision);
            drop(s);
            return self.clipboard_image_get(device, epoch, revision);
        };
        s.record_transfer(TransferDirection::ToPhone, &text, false);
        let event = protocol::ClipboardEvent {
            v: 1,
            kind: "clipboard.event",
            event_id: Uuid::new_v4(),
            origin: "windows",
            revision: s.clipboard_revision,
            text,
            automatic: false,
        };
        let _ = self.events.send(Outgoing::Wire {
            device_id: device,
            message: serde_json::to_value(event).unwrap(),
        });
        Ok(())
    }
    pub fn clipboard_set(
        &self,
        event_id: Uuid,
        text: &str,
        automatic: bool,
        base_revision: u64,
    ) -> Result<(), String> {
        let mut s = self.state.lock().unwrap();
        let consent = s.settings.realtime && s.client_sync.upload && !s.client_sync.paused;
        let duplicate = clipboard_decision(
            &s.event_set,
            event_id,
            automatic,
            base_revision,
            s.clipboard_revision,
            consent,
        )?;
        if duplicate {
            return Ok(());
        }
        let expected_sequence = automatic.then_some(s.clipboard_sequence);
        let seq = clipboard::write_checked(text, expected_sequence)?;
        s.suppressed_sequence = seq;
        s.clipboard_sequence = seq;
        s.clipboard_revision += 1;
        s.record_transfer(TransferDirection::ToPc, text, automatic);
        s.event_set.insert(event_id);
        s.event_ids.push_back(event_id);
        if s.event_ids.len() > 256 {
            if let Some(old) = s.event_ids.pop_front() {
                s.event_set.remove(&old);
            }
        }
        s.revision += 1;
        self.publish_state_locked(&s);
        Ok(())
    }
    pub fn audio_changed(&self, state: AudioState) {
        let mut s = self.state.lock().unwrap();
        s.audio = state;
        s.revision += 1;
        self.publish_state_locked(&s);
    }
    pub fn media_changed(&self) {
        let media = media::read();
        let mut s = self.state.lock().unwrap();
        s.media = media;
        s.revision += 1;
        self.publish_state_locked(&s);
    }
    pub fn diagnostic(&self, msg: &str) {
        let mut s = self.state.lock().unwrap();
        if s.diagnostics.len() == MAX_DIAGNOSTICS {
            s.diagnostics.pop_front();
        }
        s.diagnostics.push_back(DiagnosticEntry {
            at: now_ms(),
            message: msg.to_string(),
        });
    }
    pub fn set_service_running(&self, running: bool) {
        self.state.lock().unwrap().service_running = running;
    }
    pub fn check_resume(&self) {
        let now = unsafe { windows::Win32::System::SystemInformation::GetTickCount64() };
        let last = self.last_live_tick.fetch_max(now, Ordering::AcqRel);
        if now.saturating_sub(last) <= 5_000 {
            return;
        }
        let mut s = self.state.lock().unwrap();
        if let Some(device_id) = s.connected.take() {
            s.mark_seen(device_id);
            s.connection_epoch = s.connection_epoch.wrapping_add(1);
            s.clear_session_features();
            s.end_file_session();
            let _ = self.events.send(Outgoing::Close { device_id });
            drop(s);
            self.diagnostic(
                "PC retomado após pausa de execução. Conexão reiniciada sem repetir ações.",
            );
        }
    }
}

/// Sessions without the playback position, for change detection.
fn media_key(media: &MediaState) -> MediaState {
    MediaState {
        sessions: media
            .sessions
            .iter()
            .map(|session| protocol::MediaSession {
                position_seconds: None,
                ..session.clone()
            })
            .collect(),
    }
}

pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

fn clipboard_decision(
    seen: &HashSet<Uuid>,
    id: Uuid,
    automatic: bool,
    base: u64,
    current: u64,
    consent: bool,
) -> Result<bool, String> {
    if seen.contains(&id) {
        return Ok(true);
    }
    if automatic && !consent {
        return Err("unavailable".into());
    }
    if automatic && base != current {
        return Err("conflict".into());
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::Device;
    use std::net::IpAddr;

    fn ip(value: &str) -> IpAddr {
        value.parse().unwrap()
    }

    #[test]
    fn pairing_addresses_prefer_the_default_route_over_virtual_adapters() {
        let candidates = vec![ip("172.29.192.1"), ip("192.168.0.102"), ip("10.8.0.2")];
        assert_eq!(
            ordered_addresses(candidates, Some(ip("192.168.0.102"))),
            vec!["192.168.0.102", "10.8.0.2", "172.29.192.1"]
        );
    }

    #[test]
    fn pairing_addresses_fall_back_to_sorted_distinct_ipv4_addresses() {
        let candidates = vec![
            ip("172.29.192.1"),
            ip("192.168.0.102"),
            ip("172.29.192.1"),
            ip("127.0.0.1"),
            ip("169.254.1.2"),
            ip("0.0.0.0"),
            ip("fe80::1"),
        ];
        let expected = vec!["172.29.192.1", "192.168.0.102"];
        assert_eq!(ordered_addresses(candidates.clone(), None), expected);
        assert_eq!(
            ordered_addresses(candidates, Some(ip("10.9.0.1"))),
            expected
        );
    }

    #[test]
    #[ignore = "requires an active IPv4 default route on the host"]
    fn default_route_qr_invitation_native_smoke() {
        let preferred = local_ip_address::local_ip().expect("active IPv4 default route");
        let invitation = test_core().start_pairing();
        let payload: serde_json::Value = serde_json::from_str(&invitation.qr_payload).unwrap();
        assert_eq!(payload["host"], preferred.to_string());
    }

    fn test_core() -> Arc<Core> {
        Core::ephemeral(
            Secrets {
                pc_id: Uuid::new_v4(),
                certificate_pem: String::new(),
                private_key_pem: String::new(),
                devices: Vec::new(),
            },
            "00".into(),
        )
    }
    #[test]
    fn constant_time_comparison() {
        assert!(constant_time_eq(&[1, 2], &[1, 2]));
        assert!(!constant_time_eq(&[1, 2], &[1, 3]));
    }
    #[test]
    fn token_length() {
        assert_eq!(random_hex().len(), 64);
    }
    #[test]
    fn old_socket_cleanup_cannot_remove_a_new_session() {
        let core = test_core();
        let id = Uuid::new_v4();
        let token = random_hex();
        core.state.lock().unwrap().secrets.devices.push(Device {
            id,
            name: "Test".into(),
            token_hash: secrets::token_hash(&token),
        });
        let old = core
            .connect_authenticated(id, &token, Features::legacy(false, false))
            .unwrap();
        let new = core
            .connect_authenticated(id, &token, Features::legacy(false, false))
            .unwrap();
        core.disconnect(id, old);
        assert!(core.is_authorized_session(id, new));
        assert!(!core.is_authorized_session(id, old));
    }
    #[test]
    fn last_seen_follows_sessions_and_is_dropped_on_revocation() {
        let core = test_core();
        let id = Uuid::new_v4();
        let token = random_hex();
        core.state.lock().unwrap().secrets.devices.push(Device {
            id,
            name: "Test".into(),
            token_hash: secrets::token_hash(&token),
        });
        assert_eq!(core.status().devices[0].last_seen_at, None);
        let epoch = core
            .connect_authenticated(id, &token, Features::legacy(false, false))
            .unwrap();
        let connected_at = core.status().devices[0].last_seen_at.unwrap();
        core.disconnect(id, epoch);
        let device = &core.status().devices[0];
        assert!(!device.connected);
        assert!(device.last_seen_at.unwrap() >= connected_at);
        core.revoke_with_save(id, |_| Ok(())).unwrap();
        assert!(!core.state.lock().unwrap().last_seen.contains_key(&id));
    }
    #[test]
    fn transfer_history_keeps_metadata_only_and_is_bounded() {
        let core = test_core();
        {
            let mut s = core.state.lock().unwrap();
            for index in 0..(MAX_TRANSFERS + 3) {
                s.record_transfer(TransferDirection::ToPc, "segredo ✓", index % 2 == 0);
            }
            s.record_transfer(TransferDirection::ToPhone, "abc", false);
        }
        let status = core.status();
        assert_eq!(status.transfers.len(), MAX_TRANSFERS);
        // Newest first, sized in characters rather than bytes.
        assert_eq!(status.transfers[0].direction, TransferDirection::ToPhone);
        assert_eq!(status.transfers[0].characters, 3);
        assert_eq!(status.transfers[1].characters, 9);
        let wire = serde_json::to_string(&status.transfers).unwrap();
        assert!(!wire.contains("segredo"));
        assert!(wire.contains("\"toPc\""));
    }
    #[test]
    fn diagnostics_are_timestamped_and_bounded() {
        let core = test_core();
        for index in 0..(MAX_DIAGNOSTICS + 5) {
            core.diagnostic(&format!("evento {index}"));
        }
        let diagnostics = core.status().diagnostics;
        assert_eq!(diagnostics.len(), MAX_DIAGNOSTICS);
        assert_eq!(diagnostics[0].message, "evento 5");
        assert!(diagnostics.iter().all(|entry| entry.at > 0));
    }
    #[test]
    fn resume_invalidates_pending_session_without_revoking_credentials() {
        let core = test_core();
        let id = Uuid::new_v4();
        let token = random_hex();
        core.state.lock().unwrap().secrets.devices.push(Device {
            id,
            name: "Resume test".into(),
            token_hash: secrets::token_hash(&token),
        });
        let epoch = core
            .connect_authenticated(id, &token, Features::legacy(false, false))
            .unwrap();
        let now = unsafe { windows::Win32::System::SystemInformation::GetTickCount64() };
        core.last_live_tick
            .store(now.saturating_sub(6_000), Ordering::Release);
        core.check_resume();
        assert!(!core.is_authorized_session(id, epoch));
        assert!(core
            .connect_authenticated(id, &token, Features::legacy(false, false))
            .is_ok());
    }
    #[test]
    fn pairing_approval_and_revocation() {
        let core = test_core();
        let device = Uuid::new_v4();
        let code = core.start_pairing().code;
        let request = Uuid::new_v4();
        let (rx, generation) = core
            .pair_request(request, device, "Android".into(), code)
            .unwrap();
        core.approve_pairing(request, true).unwrap();
        assert!(rx.blocking_recv().unwrap());
        let (_, token) = core
            .pair_complete(device, "Android".into(), generation)
            .unwrap();
        assert!(core
            .connect_authenticated(device, &token, Features::legacy(false, false))
            .is_ok());
        assert!(core
            .connect_authenticated(device, &token, Features::legacy(false, false))
            .is_ok());
        core.revoke(device).unwrap();
        assert!(!core.is_authorized_connection(device));
        assert!(core
            .connect_authenticated(device, &token, Features::legacy(false, false))
            .is_err());
    }
    #[test]
    fn failed_revocation_closes_session_and_preserves_durable_credentials_for_retry() {
        let core = test_core();
        let id = Uuid::new_v4();
        let token = random_hex();
        core.state.lock().unwrap().secrets.devices.push(Device {
            id,
            name: "Revocation test".into(),
            token_hash: secrets::token_hash(&token),
        });
        let epoch = core
            .connect_authenticated(id, &token, Features::legacy(true, false))
            .unwrap();
        {
            let mut s = core.state.lock().unwrap();
            s.input_enabled = true;
            s.client_sync.upload = true;
            s.client_sync.download = true;
        }
        let mut events = core.events.subscribe();
        let failure = core.revoke_with_save(id, |candidate| {
            assert!(!candidate.devices.iter().any(|d| d.id == id));
            Err("disk full".into())
        });
        let error = failure.unwrap_err();
        assert!(error.contains("disk full"));
        assert!(error.contains("Acesso bloqueado nesta execução"));
        assert!(error.contains("Reiniciar"));
        assert!(matches!(events.try_recv(), Ok(Outgoing::Close { device_id }) if device_id == id));
        assert!(!core.is_authorized_session(id, epoch));
        assert_eq!(
            core.execute_input(id, epoch, &Action::PointerMove { dx: 1, dy: 0 }),
            Err("unauthorized".into())
        );
        {
            let s = core.state.lock().unwrap();
            assert_eq!(s.secrets.devices.len(), 1);
            assert!(s.connected.is_none());
            assert!(!s.input_feature && !s.input_enabled);
            assert!(!s.client_sync.upload && !s.client_sync.download);
        }
        // Disk and RAM retain the old hash, but automatic reconnection is denied.
        assert_eq!(
            core.connect_authenticated(id, &token, Features::legacy(true, false)),
            Err("unauthorized")
        );
        core.revoke_with_save(id, |_| Ok(())).unwrap();
        assert!(core
            .connect_authenticated(id, &token, Features::legacy(true, false))
            .is_err());
    }
    #[test]
    fn settings_failure_restores_autostart_and_keeps_live_preferences() {
        let core = test_core();
        let old = serde_json::to_value(&core.status().settings).unwrap();
        let mut changes = Vec::new();
        let new = Settings {
            autostart: true,
            input_blocked: true,
            ..Settings::default()
        };
        assert_eq!(
            core.update_settings_with_io(
                new,
                |value| {
                    changes.push(value);
                    Ok(())
                },
                |_| Err("settings disk failure".into()),
            ),
            Err("settings disk failure".into())
        );
        assert_eq!(changes, vec![true, false]);
        assert_eq!(serde_json::to_value(&core.status().settings).unwrap(), old);
    }
    #[test]
    fn settings_reports_rollback_failure_and_skips_unchanged_autostart() {
        let core = test_core();
        let mut calls = 0;
        let result = core.update_settings_with_io(
            Settings {
                autostart: true,
                ..Settings::default()
            },
            |_| {
                calls += 1;
                if calls == 1 {
                    Ok(())
                } else {
                    Err("registry rollback failed".into())
                }
            },
            |_| Err("settings disk failure".into()),
        );
        let error = result.unwrap_err();
        assert!(error.contains("settings disk failure"));
        assert!(error.contains("registry rollback failed"));
        assert!(!core.status().settings.autostart);
        core.update_settings_with_io(
            Settings {
                realtime: false,
                ..Settings::default()
            },
            |_| panic!("unchanged autostart must not touch registry"),
            |_| Ok(()),
        )
        .unwrap();
        assert!(!core.status().settings.realtime);
    }
    #[test]
    fn concurrent_settings_updates_serialize_persistence_and_live_publication() {
        use std::sync::mpsc;
        let core = test_core();
        let persisted = Arc::new(Mutex::new(Settings::default()));
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (second_saved_tx, second_saved_rx) = mpsc::channel();
        let first_core = core.clone();
        let first_disk = persisted.clone();
        let first = std::thread::spawn(move || {
            first_core.update_settings_with_io(
                Settings {
                    realtime: false,
                    ..Settings::default()
                },
                |_| panic!("autostart is unchanged"),
                |value| {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    *first_disk.lock().unwrap() = value.clone();
                    Ok(())
                },
            )
        });
        started_rx.recv().unwrap();
        let second_core = core.clone();
        let second_disk = persisted.clone();
        let second = std::thread::spawn(move || {
            second_core.update_settings_with_io(
                Settings {
                    realtime: true,
                    ..Settings::default()
                },
                |_| panic!("autostart is unchanged"),
                |value| {
                    second_saved_tx.send(()).unwrap();
                    *second_disk.lock().unwrap() = value.clone();
                    Ok(())
                },
            )
        });
        assert!(matches!(
            second_saved_rx.recv_timeout(std::time::Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        release_tx.send(()).unwrap();
        first.join().unwrap().unwrap();
        second.join().unwrap().unwrap();
        assert_eq!(
            serde_json::to_value(&core.status().settings).unwrap(),
            serde_json::to_value(&*persisted.lock().unwrap()).unwrap(),
        );
        // The second update replaced the whole object after the first was saved.
        assert!(core.status().settings.realtime);
    }
    #[test]
    fn clipboard_sync_follows_the_single_realtime_switch_and_the_phone_directions() {
        let core = test_core();
        let id = Uuid::new_v4();
        let token = random_hex();
        core.state.lock().unwrap().secrets.devices.push(Device {
            id,
            name: "Sync test".into(),
            token_hash: secrets::token_hash(&token),
        });
        core.connect_authenticated(id, &token, Features::legacy(true, true))
            .unwrap();
        // Automatic writes from the phone need the phone's upload direction too.
        assert_eq!(
            core.clipboard_set(Uuid::new_v4(), "x", true, 0),
            Err("unavailable".into())
        );
        core.set_client_sync(true, true, false);
        core.state.lock().unwrap().settings.realtime = false;
        assert_eq!(
            core.clipboard_set(Uuid::new_v4(), "x", true, 0),
            Err("unavailable".into())
        );
        // The PC side of the gate is only `realtime`; manual sends never depend on it.
        let s = core.state.lock().unwrap();
        assert!(!s.settings.realtime && s.client_sync.upload && s.client_sync.download);
    }
    #[test]
    fn files_are_allowed_without_a_switch_and_v2_sessions_get_one_gigabyte() {
        let core = test_core();
        let id = Uuid::new_v4();
        let token = random_hex();
        core.state.lock().unwrap().secrets.devices.push(Device {
            id,
            name: "Files limit".into(),
            token_hash: secrets::token_hash(&token),
        });
        core.connect_authenticated(id, &token, Features::legacy(false, true))
            .unwrap();
        let files = core.snapshot().files.unwrap();
        assert!(files.allowed);
        assert_eq!(files.max_bytes, protocol::MAX_FILE_BYTES);
        let features = Features {
            files: true,
            files_v2: true,
            ..Features::default()
        };
        let epoch = core.connect_authenticated(id, &token, features).unwrap();
        assert_eq!(
            core.snapshot().files.unwrap().max_bytes,
            protocol::MAX_FILE_BYTES_V2
        );
        assert!(core.files_v2(id, epoch));
        assert!(core.status().phone_receives_files);
        core.disconnect(id, epoch);
        assert!(!core.status().phone_receives_files);
    }
    #[test]
    fn input_is_ready_after_pairing_and_interromper_blocks_until_liberar() {
        let core = test_core();
        let id = Uuid::new_v4();
        let token = random_hex();
        core.state.lock().unwrap().secrets.devices.push(Device {
            id,
            name: "Input test".into(),
            token_hash: secrets::token_hash(&token),
        });
        let old = core
            .connect_authenticated(id, &token, Features::legacy(false, false))
            .unwrap();
        assert!(core.snapshot().input.is_none());
        assert_eq!(core.input_enable(id, old, true), Err("unavailable".into()));
        let fresh = core
            .connect_authenticated(id, &token, Features::legacy(true, false))
            .unwrap();
        // No PC switch any more: allowed from the first snapshot, still disarmed.
        let input = core.snapshot().input.unwrap();
        assert!(input.allowed && !input.enabled);
        assert!(!core.input_active(id, fresh));
        core.input_enable(id, fresh, true).unwrap();
        assert!(core.input_active(id, fresh));
        let mut saved = None;
        core.stop_input_with_save(|settings| {
            saved = Some(settings.clone());
            Ok(())
        })
        .unwrap();
        assert!(saved.unwrap().input_blocked);
        assert!(!core.input_active(id, fresh));
        // The session stays for media and files; only input is refused.
        assert!(core.is_authorized_session(id, fresh));
        assert!(!core.snapshot().input.unwrap().allowed);
        assert_eq!(
            core.execute_input(id, fresh, &Action::PointerMove { dx: 1, dy: 0 }),
            Err("unavailable".into())
        );
        assert_eq!(
            core.input_enable(id, fresh, true),
            Err("unavailable".into())
        );
        // A reconnection does not undo the block.
        let fresh = core
            .connect_authenticated(id, &token, Features::legacy(true, false))
            .unwrap();
        assert_eq!(
            core.input_enable(id, fresh, true),
            Err("unavailable".into())
        );
        // A failed save still blocks this run.
        let error = core
            .stop_input_with_save(|_| Err("disk".into()))
            .unwrap_err();
        assert!(error.contains("bloqueados nesta execução"));
        assert!(core.status().settings.input_blocked);
        // Settings sent by a panel that still shows the block as lifted do not lift it.
        let mut stale = core.status().settings;
        stale.input_blocked = false;
        stale.realtime = false;
        core.update_settings_with_io(stale, |_| Ok(()), |_| Ok(()))
            .unwrap();
        assert!(core.status().settings.input_blocked && !core.status().settings.realtime);
        // "Liberar" that cannot be saved keeps the block; a saved one lifts it.
        assert!(core
            .set_input_blocked_with_save(false, |_| Err("disk".into()))
            .is_err());
        assert!(core.status().settings.input_blocked);
        core.set_input_blocked_with_save(false, |_| Ok(())).unwrap();
        core.input_enable(id, fresh, true).unwrap();
        let replacement = core
            .connect_authenticated(id, &token, Features::legacy(true, false))
            .unwrap();
        assert!(!core.input_active(id, replacement));
        assert_eq!(
            core.input_enable(id, fresh, true),
            Err("unauthorized".into())
        );
        core.input_enable(id, replacement, true).unwrap();
        core.revoke(id).unwrap();
        assert!(!core.input_active(id, replacement));
        assert_eq!(
            core.execute_input(id, replacement, &Action::PointerMove { dx: 1, dy: 0 }),
            Err("unauthorized".into())
        );
    }
    #[test]
    fn old_pairing_approval_is_invalid() {
        let core = test_core();
        let first = core.start_pairing();
        let id = Uuid::new_v4();
        let (rx, generation) = core
            .pair_request(id, Uuid::new_v4(), "Phone".into(), first.code)
            .unwrap();
        core.start_pairing();
        assert!(!rx.blocking_recv().unwrap());
        assert!(core
            .pair_complete(Uuid::new_v4(), "Phone".into(), generation)
            .is_err());
    }
    #[test]
    fn clipboard_dedup_and_conflict() {
        let id = Uuid::new_v4();
        let mut seen = HashSet::new();
        assert_eq!(clipboard_decision(&seen, id, true, 4, 4, true), Ok(false));
        seen.insert(id);
        assert_eq!(clipboard_decision(&seen, id, true, 4, 5, true), Ok(true));
        assert_eq!(
            clipboard_decision(&seen, Uuid::new_v4(), true, 4, 5, true),
            Err("conflict".into())
        );
        assert_eq!(
            clipboard_decision(&seen, Uuid::new_v4(), true, 5, 5, false),
            Err("unavailable".into())
        );
        assert_eq!(
            clipboard_decision(&seen, Uuid::new_v4(), false, 0, 5, false),
            Ok(false)
        );
    }
}
