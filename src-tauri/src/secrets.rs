use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: Uuid,
    pub name: String,
    pub token_hash: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Secrets {
    pub pc_id: Uuid,
    pub certificate_pem: String,
    pub private_key_pem: String,
    pub devices: Vec<Device>,
}

pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn path() -> Result<PathBuf, String> {
    let sys = crate::platform::system();
    Ok(sys.config_dir()?.join(sys.secret_file_name()))
}

pub fn load_or_create() -> Result<Secrets, String> {
    let sys = crate::platform::system();
    let path = path()?;
    if path.exists() {
        let raw = fs::read(&path).map_err(|e| e.to_string())?;
        let plain = sys.protect_secret(&raw, true)?;
        return serde_json::from_slice(&plain).map_err(|e| e.to_string());
    }
    let key = rcgen::KeyPair::generate().map_err(|e| e.to_string())?;
    let cert = rcgen::generate_simple_self_signed(vec!["pc-remote.local".into()])
        .map_err(|e| e.to_string())?;
    // generate_simple_self_signed provides the matching signing key.
    let secrets = Secrets {
        pc_id: Uuid::new_v4(),
        certificate_pem: cert.cert.pem(),
        private_key_pem: cert.signing_key.serialize_pem(),
        devices: Vec::new(),
    };
    drop(key);
    save(&secrets)?;
    Ok(secrets)
}

pub fn save(secrets: &Secrets) -> Result<(), String> {
    let sys = crate::platform::system();
    let serialized = serde_json::to_vec(secrets).map_err(|e| e.to_string())?;
    let encrypted = sys.protect_secret(&serialized, false)?;
    let path = path()?;
    sys.save_secret_file(&path, &encrypted)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hash_is_deterministic() {
        assert_eq!(token_hash("abc"), hex::encode(Sha256::digest(b"abc")));
    }
}
