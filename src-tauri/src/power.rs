//! Keeps the PC from sleeping while a file transfer runs in either direction.
//!
//! Delegates to the active platform's `PowerBackend::acquire_wake_lock` (Win32
//! `PowerCreateRequest` on Windows, `systemd-logind` `Inhibit` on Linux).

/// Held for the duration of one transfer.
pub struct Awake(#[allow(dead_code)] Box<dyn Send>);

pub fn awake() -> Awake {
    Awake(crate::platform::power().acquire_wake_lock())
}
