use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Settings {
    pub upload: bool,
    pub download: bool,
    pub paused: bool,
    pub autostart: bool,
    #[serde(rename = "allowInput", default)]
    pub allow_input: bool,
    #[serde(rename = "allowFiles", default)]
    pub allow_files: bool,
}

fn path() -> Result<PathBuf, String> {
    let base = std::env::var_os("APPDATA").ok_or("APPDATA unavailable")?;
    let dir = PathBuf::from(base).join("PC Remote");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("settings.json"))
}
pub fn load() -> Settings {
    path()
        .ok()
        .and_then(|p| fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
pub fn save(settings: &Settings) -> Result<(), String> {
    let p = path()?;
    let tmp = p.with_extension("tmp");
    fs::write(
        &tmp,
        serde_json::to_vec(settings).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::rename(tmp, p).map_err(|e| e.to_string())
}

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    use windows_registry::{Key, CURRENT_USER};
    let key: Key = CURRENT_USER
        .create("Software\\Microsoft\\Windows\\CurrentVersion\\Run")
        .map_err(|e| e.to_string())?;
    if enabled {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        key.set_string("PC Remote", format!("\"{}\"", exe.display()))
            .map_err(|e| e.to_string())
    } else {
        match key.remove_value("PC Remote") {
            Ok(()) => Ok(()),
            Err(error)
                if error.code().0
                    == windows::core::HRESULT::from_win32(
                        windows::Win32::Foundation::ERROR_FILE_NOT_FOUND.0,
                    )
                    .0 =>
            {
                Ok(())
            }
            Err(error) => Err(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Settings;

    #[test]
    fn legacy_preferences_never_grant_remote_input_or_files() {
        let settings: Settings = serde_json::from_str(
            r#"{"upload":true,"download":false,"paused":false,"autostart":false}"#,
        )
        .unwrap();
        assert!(!settings.allow_input);
        assert!(!settings.allow_files);
        assert!(settings.upload);
        let current = serde_json::to_value(&settings).unwrap();
        assert_eq!(current["allowInput"], false);
        assert_eq!(current["allowFiles"], false);
        // 0.2.x/0.3.0 files with input already granted still keep files blocked.
        let input_only: Settings = serde_json::from_str(
            r#"{"upload":false,"download":false,"paused":false,"autostart":false,"allowInput":true}"#,
        )
        .unwrap();
        assert!(input_only.allow_input);
        assert!(!input_only.allow_files);
    }
}
