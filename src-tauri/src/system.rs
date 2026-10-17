//! The OS name the phone shows under the PC name (info.v1) and MAC addresses (wol.v1).
//! Delegates to the active platform's `SystemBackend`.

/// "Windows 11 Pro", "Ubuntu 24.04.1 LTS"… read once from the platform backend.
pub fn os_name() -> &'static str {
    crate::platform::system().os_name()
}

/// Active unicast MAC addresses (`"aa:bb:cc:dd:ee:ff"`) for Wake-on-LAN (`wol.v1`).
pub fn mac_addresses() -> Vec<String> {
    crate::platform::system().mac_addresses()
}
