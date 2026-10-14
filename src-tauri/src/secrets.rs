use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};
use uuid::Uuid;
use windows::Win32::{
    Foundation::{LocalFree, HLOCAL},
    Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    },
};

#[derive(Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: Uuid,
    pub name: String,
    pub token_hash: String,
}

#[derive(Serialize, Deserialize)]
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
    let base = std::env::var_os("APPDATA").ok_or("APPDATA unavailable")?;
    let path = PathBuf::from(base).join("PC Remote");
    fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    Ok(path.join("identity.dpapi"))
}

fn protect(data: &[u8], decrypt: bool) -> Result<Vec<u8>, String> {
    unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        let result = if decrypt {
            CryptUnprotectData(
                &input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptProtectData(
                &input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        result.map_err(|e| e.to_string())?;
        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(output.pbData as *mut _)));
        Ok(bytes)
    }
}

pub fn load_or_create() -> Result<Secrets, String> {
    let path = path()?;
    if path.exists() {
        let plain = protect(&fs::read(&path).map_err(|e| e.to_string())?, true)?;
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
    let encrypted = protect(
        &serde_json::to_vec(secrets).map_err(|e| e.to_string())?,
        false,
    )?;
    let path = path()?;
    let temp = path.with_extension("tmp");
    fs::write(&temp, encrypted).map_err(|e| e.to_string())?;
    fs::rename(&temp, &path).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hash_is_deterministic() {
        assert_eq!(token_hash("abc"), hex::encode(Sha256::digest(b"abc")));
    }
}
