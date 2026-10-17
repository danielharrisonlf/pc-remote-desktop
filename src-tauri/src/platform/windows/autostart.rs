//! Windows Registry (`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`) implementation of `AutostartBackend`.
use crate::platform::{AutostartBackend, PlatformError};
use windows_registry::{Key, CURRENT_USER};

pub struct WindowsAutostart;

impl AutostartBackend for WindowsAutostart {
    fn set_enabled(&self, enabled: bool) -> Result<(), PlatformError> {
        let key: Key = CURRENT_USER
            .create("Software\\Microsoft\\Windows\\CurrentVersion\\Run")
            .map_err(|e| PlatformError::Io(e.to_string()))?;
        if enabled {
            let exe = std::env::current_exe().map_err(|e| PlatformError::Io(e.to_string()))?;
            key.set_string("PC Remote", format!("\"{}\"", exe.display()))
                .map_err(|e| PlatformError::Io(e.to_string()))
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
                Err(error) => Err(PlatformError::Io(error.to_string())),
            }
        }
    }
}
