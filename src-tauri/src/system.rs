//! The OS name the phone shows under the PC name (info.v1).
//! Delegates to the active platform's `SystemBackend`.

/// "Windows 11 Pro", "Ubuntu 24.04.1 LTS"… read once from the platform backend.
pub fn os_name() -> &'static str {
    crate::platform::system().os_name()
}
