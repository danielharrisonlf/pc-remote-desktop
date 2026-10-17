//! Linux XDG autostart implementation (`AutostartBackend`).
//!
//! Creates or removes `~/.config/autostart/pc-remote-desktop.desktop` (respecting `$XDG_CONFIG_HOME`).
use crate::platform::{AutostartBackend, PlatformError};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub struct LinuxAutostart;

const DESKTOP_FILE_NAME: &str = "pc-remote-desktop.desktop";

pub(crate) fn autostart_file_path() -> Result<PathBuf, PlatformError> {
    let config_root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .ok_or_else(|| PlatformError::Io("XDG_CONFIG_HOME/HOME unavailable".into()))?;
    Ok(config_root.join("autostart").join(DESKTOP_FILE_NAME))
}

pub(crate) fn desktop_entry_contents(exe: &Path) -> String {
    let escaped = exe.display().to_string().replace('"', "\\\"");
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Version=1.0\n\
         Name=PC Remote\n\
         Comment=Controle o PC pelo celular na sua rede Wi-Fi\n\
         Exec=\"{escaped}\"\n\
         Icon=pc-remote-desktop\n\
         Terminal=false\n\
         Categories=Utility;RemoteAccess;\n\
         StartupNotify=false\n\
         X-GNOME-Autostart-enabled=true\n"
    )
}

impl AutostartBackend for LinuxAutostart {
    fn set_enabled(&self, enabled: bool) -> Result<(), PlatformError> {
        let path = autostart_file_path()?;
        if enabled {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| PlatformError::Io(e.to_string()))?;
            }
            let exe = std::env::current_exe().map_err(|e| PlatformError::Io(e.to_string()))?;
            let content = desktop_entry_contents(&exe);
            fs::write(&path, content).map_err(|e| PlatformError::Io(e.to_string()))
        } else {
            match fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(err) => Err(PlatformError::Io(err.to_string())),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_entry_follows_xdg_autostart_spec() {
        let entry = desktop_entry_contents(Path::new("/usr/bin/pc-remote-desktop"));
        assert!(entry.starts_with("[Desktop Entry]\n"));
        assert!(entry.contains("Type=Application\n"));
        assert!(entry.contains("Exec=\"/usr/bin/pc-remote-desktop\"\n"));
        assert!(entry.contains("Categories=Utility;RemoteAccess;\n"));
        assert!(entry.contains("X-GNOME-Autostart-enabled=true\n"));
    }
}
