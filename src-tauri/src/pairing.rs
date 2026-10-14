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

#[derive(Clone, Serialize)]
pub struct PendingView {
    pub id: Uuid,
    #[serde(rename = "deviceName")]
    pub device_name: String,
}

pub struct Pending {
    pub device_id: Uuid,
    pub device_name: String,
    pub generation: u64,
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
            || s.secrets.devices.iter().any(|d| d.id == device_id)
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
        s.secrets.devices.retain(|x| x.id != device_id);
        s.secrets.devices.push(d);
        if self.persist_secrets {
            secrets::save(&s.secrets)?;
        }
        s.pairing = None;
        for (_, p) in s.pending.drain() {
            let _ = p.response.send(false);
        }
        Ok((s.secrets.pc_id, token))
    }
}
