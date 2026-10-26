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

const REQUEST_LIFETIME: Duration = Duration::from_secs(120);
const GRANT_LIFETIME: Duration = Duration::from_secs(30);

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
    #[serde(rename = "deviceId")]
    pub device_id: Uuid,
    #[serde(rename = "deviceName")]
    pub device_name: String,
    #[serde(rename = "requestedAt")]
    pub requested_at: u64,
    #[serde(rename = "expiresAt")]
    pub expires_at: u64,
}

#[derive(Clone, Copy, Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PairingState {
    Approved,
    Completed,
    Failed,
    Denied,
    Expired,
    Invalidated,
}

#[derive(Clone, Serialize)]
pub struct PairingResult {
    #[serde(rename = "requestId")]
    pub request_id: Uuid,
    #[serde(rename = "deviceId")]
    pub device_id: Uuid,
    pub state: PairingState,
    pub message: Option<String>,
}

impl PairingResult {
    fn new(request_id: Uuid, device_id: Uuid, state: PairingState) -> Self {
        let message = match state {
            PairingState::Approved | PairingState::Completed => None,
            PairingState::Failed => Some("Não foi possível salvar o pareamento no PC"),
            PairingState::Denied => Some("Pareamento recusado no PC"),
            PairingState::Expired => Some("Solicitação de pareamento expirada"),
            PairingState::Invalidated => Some("Solicitação substituída por um novo código"),
        };
        Self {
            request_id,
            device_id,
            state,
            message: message.map(str::to_owned),
        }
    }
}

/// This value can only come from a successful local approval. Consuming it prevents
/// a second completion, and its private fields bind the approval to the request.
pub struct PairingGrant {
    request_id: Uuid,
    device_id: Uuid,
    device_name: String,
    generation: u64,
    deadline: Instant,
}

pub enum ApprovalOutcome {
    Approved(PairingGrant),
    Denied,
    Expired,
    Invalidated,
}

pub struct Pending {
    pub device_id: Uuid,
    pub device_name: String,
    pub generation: u64,
    pub requested_at: u64,
    pub expires_at: u64,
    pub deadline: Instant,
    pub response: oneshot::Sender<ApprovalOutcome>,
}

impl Core {
    pub fn start_pairing(&self) -> Pairing {
        use rand::Rng;
        let mut s = self.state.lock().unwrap();
        let approved = s.pairing_result.as_ref().and_then(|result| {
            (result.state == PairingState::Approved)
                .then_some((result.request_id, result.device_id))
        });
        let invalidated: Vec<_> = s.pending.drain().collect();
        for (id, p) in invalidated {
            s.pairing_result = Some(PairingResult::new(
                id,
                p.device_id,
                PairingState::Invalidated,
            ));
            let _ = p.response.send(ApprovalOutcome::Invalidated);
        }
        if let Some((request_id, device_id)) = approved {
            s.pairing_result = Some(PairingResult::new(
                request_id,
                device_id,
                PairingState::Invalidated,
            ));
        }
        s.pair_generation = s.pair_generation.wrapping_add(1);
        s.failed_pairs.clear();
        let code = format!("{:08}", rand::rng().random_range(0..100_000_000));
        s.addresses = addresses();
        let host = s
            .addresses
            .first()
            .cloned()
            .unwrap_or_else(|| "127.0.0.1".into());
        let expires_at = now_ms() + 120_000;
        let qr_payload = json!({"v":1,"host":host,"port":protocol::PORT,"pcId":s.secrets.pc_id,"fingerprint":s.fingerprint,"code":code}).to_string();
        let pairing = Pairing {
            code,
            expires_at,
            qr_payload,
        };
        s.pairing = Some((pairing.clone(), Instant::now() + Duration::from_secs(120)));
        pairing
    }

    pub fn pair_request(
        &self,
        id: Uuid,
        device_id: Uuid,
        device_name: String,
        code: String,
    ) -> Result<oneshot::Receiver<ApprovalOutcome>, &'static str> {
        let mut s = self.state.lock().unwrap();
        let now = Instant::now();
        // Only guesses against a live code count, and they count per code: a
        // global lockout let anyone on the network block pairing before the
        // user even opened the drawer, and a new code could not lift it.
        let Some((invitation, code_deadline)) = s.pairing.as_ref() else {
            return Err("pairing_expired");
        };
        if *code_deadline <= now {
            return Err("pairing_expired");
        }
        if invitation.code != code {
            s.failed_pairs.push_back(now);
            if s.failed_pairs.len() >= 8 {
                // Too many guesses burn the code; the PC shows it as expired.
                s.pairing = None;
                s.failed_pairs.clear();
            }
            return Err("pairing_code_incorrect");
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
        let requested_at = now_ms();
        let generation = s.pair_generation;
        s.pending.insert(
            id,
            Pending {
                device_id,
                device_name,
                generation,
                requested_at,
                expires_at: requested_at + REQUEST_LIFETIME.as_millis() as u64,
                deadline: now + REQUEST_LIFETIME,
                response: tx,
            },
        );
        s.pairing_result = None;
        Ok(rx)
    }

    pub fn cancel_pending(&self, id: Uuid) {
        self.state.lock().unwrap().pending.remove(&id);
    }

    /// Expire a request if its own lifetime has elapsed. If it is still live,
    /// return its actual deadline so a caller with an early timer can re-arm it.
    pub fn expire_pending(&self, id: Uuid) -> Option<Instant> {
        let mut s = self.state.lock().unwrap();
        let Some(deadline) = s.pending.get(&id).map(|pending| pending.deadline) else {
            return None;
        };
        if deadline > Instant::now() {
            return Some(deadline);
        }
        let pending = s.pending.remove(&id).unwrap();
        s.pairing_result = Some(PairingResult::new(
            id,
            pending.device_id,
            PairingState::Expired,
        ));
        let _ = pending.response.send(ApprovalOutcome::Expired);
        None
    }

    pub fn approve_pairing(&self, id: Uuid, approved: bool) -> Result<(), String> {
        let mut s = self.state.lock().unwrap();
        let pending = s.pending.remove(&id).ok_or("Solicitação não encontrada")?;
        let (state, outcome) = if pending.generation != s.pair_generation {
            (PairingState::Invalidated, ApprovalOutcome::Invalidated)
        } else if pending.deadline <= Instant::now() {
            (PairingState::Expired, ApprovalOutcome::Expired)
        } else if !approved {
            (PairingState::Denied, ApprovalOutcome::Denied)
        } else {
            let grant = PairingGrant {
                request_id: id,
                device_id: pending.device_id,
                device_name: pending.device_name,
                generation: pending.generation,
                deadline: Instant::now() + GRANT_LIFETIME,
            };
            (PairingState::Approved, ApprovalOutcome::Approved(grant))
        };
        s.pairing_result = Some(PairingResult::new(id, pending.device_id, state));
        let _ = pending.response.send(outcome);
        match state {
            PairingState::Expired => Err("Solicitação expirada".into()),
            PairingState::Invalidated => Err("Solicitação substituída por um novo código".into()),
            _ => Ok(()),
        }
    }

    pub fn pair_complete(&self, grant: PairingGrant) -> Result<(Uuid, String), String> {
        self.pair_complete_with_save(grant, |candidate| {
            if self.persist_secrets {
                secrets::save(candidate)?;
            }
            Ok(())
        })
    }

    fn pair_complete_with_save(
        &self,
        grant: PairingGrant,
        save: impl FnOnce(&secrets::Secrets) -> Result<(), String>,
    ) -> Result<(Uuid, String), String> {
        let mut s = self.state.lock().unwrap();
        if grant.generation != s.pair_generation {
            s.pairing_result = Some(PairingResult::new(
                grant.request_id,
                grant.device_id,
                PairingState::Invalidated,
            ));
            return Err("pairing_invalidated".into());
        }
        if grant.deadline <= Instant::now() {
            s.pairing_result = Some(PairingResult::new(
                grant.request_id,
                grant.device_id,
                PairingState::Expired,
            ));
            return Err("pairing_timeout".into());
        }
        let token = random_hex();
        let mut candidate = s.secrets.clone();
        candidate.devices.retain(|d| d.id != grant.device_id);
        candidate.devices.push(Device {
            id: grant.device_id,
            name: grant.device_name,
            token_hash: secrets::token_hash(&token),
        });
        if save(&candidate).is_err() {
            s.pairing_result = Some(PairingResult::new(
                grant.request_id,
                grant.device_id,
                PairingState::Failed,
            ));
            return Err("pairing_failed".into());
        }
        s.secrets = candidate;
        s.revoked_devices.remove(&grant.device_id);
        s.pairing = None;
        // Invalidate any other grants issued for this invitation before they can
        // complete and pair additional devices.
        s.pair_generation = s.pair_generation.wrapping_add(1);
        s.pairing_result = Some(PairingResult::new(
            grant.request_id,
            grant.device_id,
            PairingState::Completed,
        ));
        for (_, pending) in s.pending.drain() {
            let _ = pending.response.send(ApprovalOutcome::Invalidated);
        }
        Ok((s.secrets.pc_id, token))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        core::Core,
        protocol::Features,
        secrets::{self, Secrets},
    };

    fn test_core() -> std::sync::Arc<Core> {
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

    fn request(
        core: &Core,
        device_id: Uuid,
    ) -> (Pairing, Uuid, oneshot::Receiver<ApprovalOutcome>) {
        let invitation = core.start_pairing();
        let request_id = Uuid::new_v4();
        let response = core
            .pair_request(
                request_id,
                device_id,
                "Test phone".into(),
                invitation.code.clone(),
            )
            .unwrap();
        (invitation, request_id, response)
    }

    fn approved_grant_from_response(
        core: &Core,
        request_id: Uuid,
        response: oneshot::Receiver<ApprovalOutcome>,
    ) -> PairingGrant {
        core.approve_pairing(request_id, true).unwrap();
        match response.blocking_recv().unwrap() {
            ApprovalOutcome::Approved(grant) => grant,
            _ => panic!("approval expected"),
        }
    }

    #[test]
    fn wrong_code_is_distinguished_without_consuming_the_active_invitation() {
        let core = test_core();
        let invitation = core.start_pairing();
        assert!(matches!(
            core.pair_request(
                Uuid::new_v4(),
                Uuid::new_v4(),
                "Phone".into(),
                "incorrect".into(),
            ),
            Err("pairing_code_incorrect")
        ));
        let request_id = Uuid::new_v4();
        let device_id = Uuid::new_v4();
        let response = core
            .pair_request(request_id, device_id, "Phone".into(), invitation.code)
            .unwrap();
        let status = serde_json::to_value(core.status()).unwrap();
        assert_eq!(status["pending"][0]["deviceId"], device_id.to_string());
        assert!(status["pending"][0]["expiresAt"].as_u64().is_some());
        core.approve_pairing(request_id, false).unwrap();
        assert!(matches!(
            response.blocking_recv().unwrap(),
            ApprovalOutcome::Denied
        ));
        assert_eq!(
            core.state
                .lock()
                .unwrap()
                .pairing_result
                .as_ref()
                .unwrap()
                .state,
            PairingState::Denied
        );
        let status = serde_json::to_value(core.status()).unwrap();
        assert_eq!(status["pairingResult"]["requestId"], request_id.to_string());
        assert_eq!(status["pairingResult"]["deviceId"], device_id.to_string());
        assert_eq!(status["pairingResult"]["state"], "denied");
        assert!(status["pairingResult"]["message"].as_str().is_some());
    }

    #[test]
    fn expired_invitation_and_rate_limit_are_enforced() {
        let core = test_core();
        let invitation = core.start_pairing();
        core.state.lock().unwrap().pairing.as_mut().unwrap().1 =
            Instant::now() - Duration::from_secs(1);
        assert!(matches!(
            core.pair_request(
                Uuid::new_v4(),
                Uuid::new_v4(),
                "Phone".into(),
                invitation.code,
            ),
            Err("pairing_expired")
        ));

        let core = test_core();
        let invitation = core.start_pairing();
        for _ in 0..8 {
            assert!(matches!(
                core.pair_request(
                    Uuid::new_v4(),
                    Uuid::new_v4(),
                    "Phone".into(),
                    "incorrect".into(),
                ),
                Err("pairing_code_incorrect")
            ));
        }
        assert!(matches!(
            core.pair_request(
                Uuid::new_v4(),
                Uuid::new_v4(),
                "Phone".into(),
                invitation.code,
            ),
            Err("pairing_expired")
        ));
        assert!(core.status().pairing.is_none());

        // A new code starts with a clean count.
        let invitation = core.start_pairing();
        assert!(core
            .pair_request(
                Uuid::new_v4(),
                Uuid::new_v4(),
                "Phone".into(),
                invitation.code,
            )
            .is_ok());
    }

    #[test]
    fn guesses_without_a_live_code_do_not_block_the_next_one() {
        let core = test_core();
        for _ in 0..16 {
            assert!(matches!(
                core.pair_request(
                    Uuid::new_v4(),
                    Uuid::new_v4(),
                    "Phone".into(),
                    "00000000".into(),
                ),
                Err("pairing_expired")
            ));
        }
        let invitation = core.start_pairing();
        assert!(core
            .pair_request(
                Uuid::new_v4(),
                Uuid::new_v4(),
                "Phone".into(),
                invitation.code,
            )
            .is_ok());
    }

    #[test]
    fn accepted_request_can_finish_after_its_invitation_code_expires() {
        let core = test_core();
        let device_id = Uuid::new_v4();
        let (_, request_id, response) = request(&core, device_id);
        core.state.lock().unwrap().pairing.as_mut().unwrap().1 =
            Instant::now() - Duration::from_secs(1);

        let grant = approved_grant_from_response(&core, request_id, response);
        let (_, token) = core.pair_complete(grant).unwrap();
        assert!(core
            .connect_authenticated(device_id, &token, Features::legacy(false, false))
            .is_ok());
    }

    #[test]
    fn expire_pending_does_not_expire_a_live_request() {
        let core = test_core();
        let (_, request_id, response) = request(&core, Uuid::new_v4());
        assert!(core.expire_pending(request_id).is_some());
        assert!(core.state.lock().unwrap().pending.contains_key(&request_id));

        core.state
            .lock()
            .unwrap()
            .pending
            .get_mut(&request_id)
            .unwrap()
            .deadline = Instant::now() - Duration::from_secs(1);
        assert!(core.expire_pending(request_id).is_none());
        assert!(matches!(
            response.blocking_recv().unwrap(),
            ApprovalOutcome::Expired
        ));
        assert_eq!(
            core.status().pairing_result.unwrap().state,
            PairingState::Expired
        );
    }

    #[test]
    fn expired_request_is_not_approved_and_new_code_invalidates_old_requests() {
        let core = test_core();
        let (_, expired_id, expired_response) = request(&core, Uuid::new_v4());
        core.state
            .lock()
            .unwrap()
            .pending
            .get_mut(&expired_id)
            .unwrap()
            .deadline = Instant::now() - Duration::from_secs(1);
        assert_eq!(
            core.approve_pairing(expired_id, true),
            Err("Solicitação expirada".into())
        );
        assert!(matches!(
            expired_response.blocking_recv().unwrap(),
            ApprovalOutcome::Expired
        ));

        let (_, invalidated_id, invalidated_response) = request(&core, Uuid::new_v4());
        core.start_pairing();
        assert!(matches!(
            invalidated_response.blocking_recv().unwrap(),
            ApprovalOutcome::Invalidated
        ));
        assert_eq!(
            core.status().pairing_result.unwrap().state,
            PairingState::Invalidated
        );
        assert!(!core
            .state
            .lock()
            .unwrap()
            .pending
            .contains_key(&invalidated_id));
    }

    #[test]
    fn regeneration_invalidates_an_approved_grant_and_grant_deadline_is_enforced() {
        let core = test_core();
        let (_, request_id, response) = request(&core, Uuid::new_v4());
        let grant = approved_grant_from_response(&core, request_id, response);
        core.start_pairing();
        assert_eq!(
            core.pair_complete(grant).unwrap_err(),
            "pairing_invalidated"
        );
        assert_eq!(
            core.status().pairing_result.unwrap().state,
            PairingState::Invalidated
        );

        let core = test_core();
        let (_, request_id, response) = request(&core, Uuid::new_v4());
        let mut grant = approved_grant_from_response(&core, request_id, response);
        grant.deadline = Instant::now() - Duration::from_secs(1);
        assert_eq!(core.pair_complete(grant).unwrap_err(), "pairing_timeout");
        assert_eq!(
            core.status().pairing_result.unwrap().state,
            PairingState::Expired
        );
    }

    #[test]
    fn one_successful_grant_invalidates_other_approvals_for_the_same_invitation() {
        let core = test_core();
        let invitation = core.start_pairing();
        let first_device = Uuid::new_v4();
        let second_device = Uuid::new_v4();
        let first_id = Uuid::new_v4();
        let second_id = Uuid::new_v4();
        let first_response = core
            .pair_request(
                first_id,
                first_device,
                "First".into(),
                invitation.code.clone(),
            )
            .unwrap();
        let second_response = core
            .pair_request(second_id, second_device, "Second".into(), invitation.code)
            .unwrap();
        let first_grant = approved_grant_from_response(&core, first_id, first_response);
        let second_grant = approved_grant_from_response(&core, second_id, second_response);

        core.pair_complete(first_grant).unwrap();
        assert_eq!(
            core.pair_complete(second_grant).unwrap_err(),
            "pairing_invalidated"
        );
    }

    #[test]
    fn pairing_save_failure_preserves_credentials_and_allows_a_clean_retry() {
        let core = test_core();
        let old_id = Uuid::new_v4();
        let old_token = "a".repeat(64);
        core.state.lock().unwrap().secrets.devices.push(Device {
            id: old_id,
            name: "Existing phone".into(),
            token_hash: secrets::token_hash(&old_token),
        });
        core.state.lock().unwrap().revoked_devices.insert(old_id);
        let invitation = core.start_pairing();
        let request_id = Uuid::new_v4();
        let response = core
            .pair_request(
                request_id,
                old_id,
                "New phone".into(),
                invitation.code.clone(),
            )
            .unwrap();
        let grant = approved_grant_from_response(&core, request_id, response);
        let result = core.pair_complete_with_save(grant, |candidate| {
            assert_eq!(candidate.devices.len(), 1);
            assert_eq!(candidate.devices[0].id, old_id);
            assert_ne!(
                candidate.devices[0].token_hash,
                secrets::token_hash(&old_token)
            );
            Err("identity disk failure".into())
        });
        assert_eq!(result, Err("pairing_failed".into()));
        {
            let state = core.state.lock().unwrap();
            assert_eq!(state.secrets.devices.len(), 1);
            assert_eq!(state.secrets.devices[0].id, old_id);
            assert_eq!(
                state.secrets.devices[0].token_hash,
                secrets::token_hash(&old_token)
            );
            assert!(state.pairing.is_some());
            assert!(state.revoked_devices.contains(&old_id));
        }
        assert_eq!(
            core.status().pairing_result.unwrap().state,
            PairingState::Failed
        );
        assert_eq!(
            core.connect_authenticated(old_id, &old_token, Features::legacy(false, false)),
            Err("unauthorized")
        );

        let retry_id = Uuid::new_v4();
        let retry_response = core
            .pair_request(retry_id, old_id, "New phone".into(), invitation.code)
            .unwrap();
        let retry_grant = approved_grant_from_response(&core, retry_id, retry_response);
        let mut persisted_token_hash = None;
        let (_, new_token) = core
            .pair_complete_with_save(retry_grant, |candidate| {
                persisted_token_hash = candidate
                    .devices
                    .iter()
                    .find(|device| device.id == old_id)
                    .map(|device| device.token_hash.clone());
                Ok(())
            })
            .unwrap();
        assert_eq!(persisted_token_hash, Some(secrets::token_hash(&new_token)));
        let state = core.state.lock().unwrap();
        assert!(state.pairing.is_none());
        assert!(!state.revoked_devices.contains(&old_id));
        drop(state);
        assert_eq!(
            core.connect_authenticated(old_id, &old_token, Features::legacy(false, false)),
            Err("unauthorized")
        );
        assert!(core
            .connect_authenticated(old_id, &new_token, Features::legacy(false, false))
            .is_ok());
    }
}
