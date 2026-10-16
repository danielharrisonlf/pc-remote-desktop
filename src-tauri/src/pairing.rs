use crate::{
    core::{addresses, now_ms, random_hex, Core},
    protocol,
    secrets::{self, Device},
};
use serde::Serialize;
use serde_json::json;
use std::time::{Duration, Instant};
use tokio::sync::oneshot;
use uuid::Uuid;

#[derive(Clone, Serialize)]
pub struct Pairing {
    pub code: String,
    #[serde(rename = "expiresAt")]
    pub expires_at: u64,
    #[serde(rename = "qrPayload")]
    pub qr_payload: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::Secrets;

    #[test]
    fn pairing_save_failure_preserves_credentials_and_allows_a_clean_retry() {
        let core = Core::ephemeral(
            Secrets {
                pc_id: Uuid::new_v4(),
                certificate_pem: String::new(),
                private_key_pem: String::new(),
                devices: Vec::new(),
            },
            "00".into(),
        );
        let old_id = Uuid::new_v4();
        let old_token = "a".repeat(64);
        core.state.lock().unwrap().secrets.devices.push(Device {
            id: old_id,
            name: "Existing phone".into(),
            token_hash: secrets::token_hash(&old_token),
        });
        core.state.lock().unwrap().revoked_devices.insert(old_id);
        let invitation = core.start_pairing();
        let new_id = old_id;
        let request = Uuid::new_v4();
        let (approval, generation) = core
            .pair_request(request, new_id, "New phone".into(), invitation.code)
            .unwrap();
        core.approve_pairing(request, true).unwrap();
        assert!(approval.blocking_recv().unwrap());
        let result =
            core.pair_complete_with_save(new_id, "New phone".into(), generation, |candidate| {
                assert_eq!(candidate.devices.len(), 1);
                assert!(candidate.devices.iter().any(|d| d.id == new_id));
                Err("identity disk failure".into())
            });
        assert_eq!(result, Err("identity disk failure".into()));
        {
            let s = core.state.lock().unwrap();
            assert_eq!(s.secrets.devices.len(), 1);
            assert_eq!(s.secrets.devices[0].id, old_id);
            assert_eq!(
                s.secrets.devices[0].token_hash,
                secrets::token_hash(&old_token)
            );
            assert!(s.pairing.is_some());
            assert!(s.revoked_devices.contains(&old_id));
        }
        assert_eq!(
            core.connect_authenticated(old_id, &old_token, false),
            Err("unauthorized")
        );
        let mut persisted_token_hash = None;
        let (_, token) = core
            .pair_complete_with_save(new_id, "New phone".into(), generation, |candidate| {
                persisted_token_hash = candidate
                    .devices
                    .iter()
                    .find(|d| d.id == new_id)
                    .map(|d| d.token_hash.clone());
                Ok(())
            })
            .unwrap();
        assert_eq!(persisted_token_hash, Some(secrets::token_hash(&token)));
        assert!(core.state.lock().unwrap().pairing.is_none());
        assert!(!core.state.lock().unwrap().revoked_devices.contains(&new_id));
        assert_eq!(
            core.connect_authenticated(old_id, &old_token, false),
            Err("unauthorized")
        );
        assert!(core.connect_authenticated(new_id, &token, false).is_ok());
    }
}

#[derive(Clone, Serialize)]
pub struct PendingView {
    pub id: Uuid,
    #[serde(rename = "deviceName")]
    pub device_name: String,
    #[serde(rename = "requestedAt")]
    pub requested_at: u64,
}

pub struct Pending {
    pub device_id: Uuid,
    pub device_name: String,
    pub generation: u64,
    pub requested_at: u64,
    pub response: oneshot::Sender<bool>,
}

impl Core {
    pub fn start_pairing(&self) -> Pairing {
        use rand::Rng;
        let mut s = self.state.lock().unwrap();
        for (_, p) in s.pending.drain() {
            let _ = p.response.send(false);
        }
        s.pair_generation = s.pair_generation.wrapping_add(1);
        let code = format!("{:08}", rand::rng().random_range(0..100_000_000));
        s.addresses = addresses();
        let host = s
            .addresses
            .first()
            .cloned()
            .unwrap_or_else(|| "127.0.0.1".into());
        let expires_at = now_ms() + 120_000;
        let qr_payload=json!({"v":1,"host":host,"port":protocol::PORT,"pcId":s.secrets.pc_id,"fingerprint":s.fingerprint,"code":code}).to_string();
        let p = Pairing {
            code,
            expires_at,
            qr_payload,
        };
        s.pairing = Some((p.clone(), Instant::now() + Duration::from_secs(120)));
        p
    }
    pub fn pair_request(
        &self,
        id: Uuid,
        device_id: Uuid,
        device_name: String,
        code: String,
    ) -> Result<(oneshot::Receiver<bool>, u64), &'static str> {
        let mut s = self.state.lock().unwrap();
        let now = Instant::now();
        while s
            .failed_pairs
            .front()
            .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(120))
        {
            s.failed_pairs.pop_front();
        }
        if s.failed_pairs.len() >= 8 {
            return Err("rate_limited");
        }
        let valid = s
            .pairing
            .as_ref()
            .is_some_and(|(p, t)| *t > now && p.code == code);
        if !valid {
            s.failed_pairs.push_back(now);
            return Err("pairing_expired");
        }
        if id.is_nil()
            || device_id.is_nil()
            || device_name.trim().is_empty()
            || device_name.len() > 80
            || s.pending.len() >= 4
            || s.pending.contains_key(&id)
            || (s.secrets.devices.iter().any(|d| d.id == device_id)
                && !s.revoked_devices.contains(&device_id))
            || s.pending.values().any(|p| p.device_id == device_id)
        {
            return Err("invalid_message");
        }
        let (tx, rx) = oneshot::channel();
        let generation = s.pair_generation;
        s.pending.insert(
            id,
            Pending {
                device_id,
                device_name,
                generation,
                requested_at: now_ms(),
                response: tx,
            },
        );
        Ok((rx, generation))
    }
    pub fn cancel_pending(&self, id: Uuid) {
        self.state.lock().unwrap().pending.remove(&id);
    }
    pub fn approve_pairing(&self, id: Uuid, approved: bool) -> Result<(), String> {
        let mut s = self.state.lock().unwrap();
        let pending = s.pending.remove(&id).ok_or("Solicitação não encontrada")?;
        let valid = s
            .pairing
            .as_ref()
            .is_some_and(|(_, deadline)| *deadline > Instant::now())
            && pending.generation == s.pair_generation;
        let _ = pending.response.send(approved && valid);
        if valid {
            Ok(())
        } else {
            Err("Código expirado".into())
        }
    }
    pub fn pair_complete(
        &self,
        device_id: Uuid,
        device_name: String,
        generation: u64,
    ) -> Result<(Uuid, String), String> {
        self.pair_complete_with_save(device_id, device_name, generation, |candidate| {
            if self.persist_secrets {
                secrets::save(candidate)?;
            }
            Ok(())
        })
    }
    fn pair_complete_with_save(
        &self,
        device_id: Uuid,
        device_name: String,
        generation: u64,
        save: impl FnOnce(&secrets::Secrets) -> Result<(), String>,
    ) -> Result<(Uuid, String), String> {
        let mut s = self.state.lock().unwrap();
        if generation != s.pair_generation
            || !s
                .pairing
                .as_ref()
                .is_some_and(|(_, deadline)| *deadline > Instant::now())
        {
            return Err("pairing_expired".into());
        }
        let token = random_hex();
        let d = Device {
            id: device_id,
            name: device_name,
            token_hash: secrets::token_hash(&token),
        };
        let mut candidate = s.secrets.clone();
        candidate.devices.retain(|x| x.id != device_id);
        candidate.devices.push(d);
        save(&candidate)?;
        s.secrets = candidate;
        s.revoked_devices.remove(&device_id);
        s.pairing = None;
        for (_, p) in s.pending.drain() {
            let _ = p.response.send(false);
        }
        Ok((s.secrets.pc_id, token))
    }
}
