pub use crate::pairing::{Pairing, Pending, PendingView};
use crate::{
    audio, clipboard, media,
    protocol::{self, AudioState, MediaState, StateSnapshot, SyncSettings},
    secrets::{self, Secrets},
    settings::{self, Settings},
};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
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
}
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
    pub diagnostics: Vec<String>,
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
    pub audio: AudioState,
    pub media: MediaState,
    pub revision: u64,
    pub clipboard_revision: u64,
    pub clipboard_sequence: u32,
    pub suppressed_sequence: u32,
    pub event_ids: VecDeque<Uuid>,
    pub event_set: HashSet<Uuid>,
    pub client_sync: SyncSettings,
    pub diagnostics: VecDeque<String>,
}
pub struct Core {
    last_live_tick: AtomicU64,
    pub state: Mutex<Runtime>,
    pub events: broadcast::Sender<Outgoing>,
    pub(crate) persist_secrets: bool,
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
    let mut a = local_ip_address::list_afinet_netifas()
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, ip)| ip.is_ipv4() && !ip.is_loopback())
        .map(|(_, ip)| ip.to_string())
        .filter(|s| !s.starts_with("127.") && !s.starts_with("169.254."))
        .collect::<Vec<_>>();
    a.sort();
    a.dedup();
    a
}

impl Core {
    #[cfg(test)]
    pub(crate) fn ephemeral(secrets: Secrets, fingerprint: String) -> Arc<Self> {
        let core = Self::build(secrets, fingerprint, false);
        core.state.lock().unwrap().settings = Settings::default();
        core
    }
    pub fn new(secrets: Secrets, fingerprint: String) -> Arc<Self> {
        Self::build(secrets, fingerprint, true)
    }
    fn build(secrets: Secrets, fingerprint: String, persist_secrets: bool) -> Arc<Self> {
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
                audio: audio::read(),
                media: media::read(),
                revision: 0,
                clipboard_revision: 0,
                clipboard_sequence: clipboard::sequence(),
                suppressed_sequence: 0,
                event_ids: VecDeque::new(),
                event_set: HashSet::new(),
                client_sync: SyncSettings::default(),
                diagnostics: VecDeque::new(),
            }),
            events,
            persist_secrets,
        })
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
                })
                .collect(),
            pending: s
                .pending
                .iter()
                .map(|(id, p)| PendingView {
                    id: *id,
                    device_name: p.device_name.clone(),
                })
                .collect(),
            settings: s.settings.clone(),
            diagnostics: s.diagnostics.iter().cloned().collect(),
        }
    }
    pub fn connect_authenticated(&self, id: Uuid, token: &str) -> Result<u64, &'static str> {
        if id.is_nil() || token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("unauthorized");
        }
        let hash = Sha256::digest(token.as_bytes());
        let mut s = self.state.lock().unwrap();
        if !s.secrets.devices.iter().any(|d| {
            d.id == id
                && hex::decode(&d.token_hash)
                    .ok()
                    .is_some_and(|h| constant_time_eq(&h, &hash))
        }) {
            return Err("unauthorized");
        }
        if s.connected.is_some_and(|current| current != id) {
            return Err("unavailable");
        }
        if s.connected == Some(id) {
            let _ = self.events.send(Outgoing::Close { device_id: id });
        }
        s.connected = Some(id);
        s.connection_epoch = s.connection_epoch.wrapping_add(1);
        s.client_sync = SyncSettings::default();
        Ok(s.connection_epoch)
    }
    pub fn is_authorized_session(&self, id: Uuid, epoch: u64) -> bool {
        let s = self.state.lock().unwrap();
        s.connected == Some(id)
            && s.connection_epoch == epoch
            && s.secrets.devices.iter().any(|d| d.id == id)
    }
    #[cfg(test)]
    pub fn is_authorized_connection(&self, id: Uuid) -> bool {
        let s = self.state.lock().unwrap();
        s.connected == Some(id) && s.secrets.devices.iter().any(|d| d.id == id)
    }
    pub fn disconnect(&self, id: Uuid, epoch: u64) {
        let mut s = self.state.lock().unwrap();
        if s.connected == Some(id) && s.connection_epoch == epoch {
            s.connected = None;
            s.client_sync = SyncSettings::default();
        }
    }
    pub fn revoke(&self, id: Uuid) -> Result<(), String> {
        let mut s = self.state.lock().unwrap();
        let count = s.secrets.devices.len();
        s.secrets.devices.retain(|d| d.id != id);
        if s.secrets.devices.len() == count {
            return Err("Dispositivo não encontrado".into());
        }
        if self.persist_secrets {
            secrets::save(&s.secrets)?;
        }
        if s.connected == Some(id) {
            s.connected = None;
            let _ = self.events.send(Outgoing::Close { device_id: id });
        }
        Ok(())
    }
    pub fn update_settings(&self, new: Settings) -> Result<(), String> {
        settings::set_autostart(new.autostart)?;
        settings::save(&new)?;
        let mut s = self.state.lock().unwrap();
        s.settings = new;
        s.revision += 1;
        self.publish_state_locked(&s);
        Ok(())
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
        }
    }
    fn publish_state_locked(&self, s: &Runtime) {
        if let Some(id) = s.connected {
            let _ = self.events.send(Outgoing::Wire {
                device_id: id,
                message: serde_json::to_value(Self::snapshot_locked(s)).unwrap(),
            });
        }
    }
    pub fn refresh(&self) {
        let expected_revision = self.state.lock().unwrap().revision;
        let audio = audio::read();
        let media = media::read();
        let mut s = self.state.lock().unwrap();
        if s.revision != expected_revision {
            return;
        }
        let before = serde_json::to_string(&(&s.audio, &s.media)).unwrap_or_default();
        let after = serde_json::to_string(&(&audio, &media)).unwrap_or_default();
        if before != after {
            s.audio = audio;
            s.media = media;
            s.revision += 1;
            self.publish_state_locked(&s);
        }
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
    pub fn clipboard_event_local(&self, sequence: u32) {
        let mut s = self.state.lock().unwrap();
        if sequence == s.clipboard_sequence || sequence == s.suppressed_sequence {
            return;
        }
        if clipboard::sequence() != sequence {
            return;
        }
        let permitted = s.settings.upload
            && !s.settings.paused
            && s.client_sync.download
            && !s.client_sync.paused;
        let text = if permitted && s.connected.is_some() {
            clipboard::read().ok().flatten()
        } else {
            None
        };
        if clipboard::sequence() != sequence {
            return;
        }
        s.clipboard_sequence = sequence;
        s.clipboard_revision += 1;
        s.revision += 1;
        self.publish_state_locked(&s);
        if let (Some(device_id), Some(text)) = (s.connected, text) {
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
        let s = self.state.lock().unwrap();
        let before = clipboard::sequence();
        let text = clipboard::read()?.ok_or("unavailable")?;
        if clipboard::sequence() != before {
            return Err("clipboard_busy".into());
        }
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
        let consent = s.settings.download
            && !s.settings.paused
            && s.client_sync.upload
            && !s.client_sync.paused;
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
        let seq = clipboard::write(text)?;
        s.suppressed_sequence = seq;
        s.clipboard_sequence = seq;
        s.clipboard_revision += 1;
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
        if s.diagnostics.len() == 20 {
            s.diagnostics.pop_front();
        }
        s.diagnostics.push_back(msg.to_string());
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
            s.connection_epoch = s.connection_epoch.wrapping_add(1);
            s.client_sync = SyncSettings::default();
            let _ = self.events.send(Outgoing::Close { device_id });
            drop(s);
            self.diagnostic(
                "PC retomado após pausa de execução. Conexão reiniciada sem repetir ações.",
            );
        }
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
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
    fn test_core() -> Arc<Core> {
        Core::build(
            Secrets {
                pc_id: Uuid::new_v4(),
                certificate_pem: String::new(),
                private_key_pem: String::new(),
                devices: Vec::new(),
            },
            "00".into(),
            false,
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
        let old = core.connect_authenticated(id, &token).unwrap();
        let new = core.connect_authenticated(id, &token).unwrap();
        core.disconnect(id, old);
        assert!(core.is_authorized_session(id, new));
        assert!(!core.is_authorized_session(id, old));
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
        let epoch = core.connect_authenticated(id, &token).unwrap();
        let now = unsafe { windows::Win32::System::SystemInformation::GetTickCount64() };
        core.last_live_tick
            .store(now.saturating_sub(6_000), Ordering::Release);
        core.check_resume();
        assert!(!core.is_authorized_session(id, epoch));
        assert!(core.connect_authenticated(id, &token).is_ok());
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
        assert!(core.connect_authenticated(device, &token).is_ok());
        assert!(core.connect_authenticated(device, &token).is_ok());
        core.revoke(device).unwrap();
        assert!(!core.is_authorized_connection(device));
        assert!(core.connect_authenticated(device, &token).is_err());
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
