use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

/// Preferences shown in the panel. Communication features need no switch:
/// files and mouse/keyboard work after pairing, and clipboard sync follows `realtime`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Settings {
    /// Automatic clipboard sync in both directions. Manual actions never depend on it.
    pub realtime: bool,
    pub autostart: bool,
    /// Set by "Interromper"; only "Liberar" in the panel clears it.
    #[serde(rename = "inputBlocked")]
    pub input_blocked: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            realtime: true,
            autostart: false,
            input_blocked: false,
        }
    }
}

fn yes() -> bool {
    true
}

/// `settings.json`. Versions up to 0.5 read only the legacy fields, so they are
/// written from the current ones; reading ignores them and takes the 0.6 defaults.
#[derive(Serialize, Deserialize)]
struct Stored {
    #[serde(default = "yes")]
    realtime: bool,
    #[serde(default)]
    autostart: bool,
    #[serde(rename = "inputBlocked", default)]
    input_blocked: bool,
    #[serde(default)]
    upload: bool,
    #[serde(default)]
    download: bool,
    #[serde(default)]
    paused: bool,
    #[serde(rename = "allowInput", default)]
    allow_input: bool,
    #[serde(rename = "allowFiles", default)]
    allow_files: bool,
}

impl From<Stored> for Settings {
    fn from(stored: Stored) -> Self {
        Self {
            realtime: stored.realtime,
            autostart: stored.autostart,
            input_blocked: stored.input_blocked,
        }
    }
}

impl From<&Settings> for Stored {
    fn from(settings: &Settings) -> Self {
        Self {
            realtime: settings.realtime,
            autostart: settings.autostart,
            input_blocked: settings.input_blocked,
            upload: settings.realtime,
            download: settings.realtime,
            paused: false,
            allow_input: !settings.input_blocked,
            allow_files: true,
        }
    }
}

fn parse(bytes: &[u8]) -> Option<Settings> {
    serde_json::from_slice::<Stored>(bytes)
        .ok()
        .map(Settings::from)
}

fn encode(settings: &Settings) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&Stored::from(settings)).map_err(|e| e.to_string())
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
        .and_then(|b| parse(&b))
        .unwrap_or_default()
}
pub fn save(settings: &Settings) -> Result<(), String> {
    let p = path()?;
    let tmp = p.with_extension("tmp");
    fs::write(&tmp, encode(settings)?).map_err(|e| e.to_string())?;
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
    use super::*;

    #[test]
    fn older_preferences_upgrade_to_the_0_6_defaults() {
        // 0.5 files had sync, files and input switched off; 0.6 turns everything on.
        for legacy in [
            r#"{"upload":false,"download":false,"paused":true,"autostart":true,"allowInput":false,"allowFiles":false}"#,
            r#"{"upload":true,"download":false,"paused":false,"autostart":true}"#,
        ] {
            let settings = parse(legacy.as_bytes()).unwrap();
            assert_eq!(
                settings,
                Settings {
                    realtime: true,
                    autostart: true,
                    input_blocked: false
                }
            );
        }
        assert_eq!(parse(b"{}").unwrap(), Settings::default());
        assert!(parse(b"nao e json").is_none());
    }

    #[test]
    fn current_fields_round_trip_and_mirror_legacy_fields_for_downgrades() {
        let settings = Settings {
            realtime: false,
            autostart: true,
            input_blocked: true,
        };
        let bytes = encode(&settings).unwrap();
        assert_eq!(parse(&bytes).unwrap(), settings);
        let stored: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(stored["upload"], false);
        assert_eq!(stored["download"], false);
        assert_eq!(stored["paused"], false);
        assert_eq!(stored["allowInput"], false);
        assert_eq!(stored["allowFiles"], true);
        let on: serde_json::Value =
            serde_json::from_slice(&encode(&Settings::default()).unwrap()).unwrap();
        assert_eq!(on["upload"], true);
        assert_eq!(on["download"], true);
        assert_eq!(on["allowInput"], true);
    }

    #[test]
    fn the_panel_sees_only_the_current_fields() {
        let panel = serde_json::to_value(Settings::default()).unwrap();
        let keys: Vec<_> = panel.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys.len(), 3);
        assert!(keys.contains(&"realtime".to_string()));
        assert!(keys.contains(&"inputBlocked".to_string()));
        assert!(keys.contains(&"autostart".to_string()));
    }
}
