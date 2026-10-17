//! Windows power & session management (`LockWorkStation`, `SetSuspendState`, `InitiateShutdownW`,
//! `PowerCreateRequest`) implementing `PowerBackend`.
use crate::platform::{PlatformError, PowerBackend};
use std::sync::Mutex;
use windows::{
    core::{PCWSTR, PWSTR},
    Win32::{
        Foundation::{
            CloseHandle, GetLastError, ERROR_NOT_ALL_ASSIGNED, HANDLE, LPARAM, LUID, WPARAM,
        },
        Security::{
            AdjustTokenPrivileges, LookupPrivilegeValueW, LUID_AND_ATTRIBUTES,
            SE_PRIVILEGE_ENABLED, SE_SHUTDOWN_NAME, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES,
            TOKEN_QUERY,
        },
        System::{
            Power::{
                GetPwrCapabilities, PowerClearRequest, PowerCreateRequest,
                PowerRequestSystemRequired, PowerSetRequest, SetSuspendState,
                SYSTEM_POWER_CAPABILITIES,
            },
            Shutdown::{
                InitiateShutdownW, LockWorkStation, SHTDN_REASON_FLAG_PLANNED,
                SHTDN_REASON_MAJOR_OTHER, SHTDN_REASON_MINOR_OTHER, SHUTDOWN_FLAGS,
                SHUTDOWN_HYBRID, SHUTDOWN_POWEROFF, SHUTDOWN_REASON, SHUTDOWN_RESTART,
                SHUTDOWN_RESTARTAPPS,
            },
            Threading::{
                GetCurrentProcess, OpenProcessToken, POWER_REQUEST_CONTEXT_SIMPLE_STRING,
                REASON_CONTEXT, REASON_CONTEXT_0,
            },
        },
        UI::WindowsAndMessaging::{
            SendMessageTimeoutW, HWND_BROADCAST, SC_MONITORPOWER, SMTO_ABORTIFHUNG, WM_SYSCOMMAND,
        },
    },
};

pub struct WindowsPower;

fn capabilities() -> Option<SYSTEM_POWER_CAPABILITIES> {
    let mut capabilities = SYSTEM_POWER_CAPABILITIES::default();
    unsafe { GetPwrCapabilities(&mut capabilities) }.then_some(capabilities)
}

/// SetSuspendState and InitiateShutdownW need SeShutdownPrivilege enabled in the token.
pub(crate) fn enable_shutdown_privilege() -> Result<(), String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
        .map_err(|e| e.message())?;
        let mut luid = LUID::default();
        let result = LookupPrivilegeValueW(PCWSTR::null(), SE_SHUTDOWN_NAME, &mut luid)
            .and_then(|()| {
                let privileges = TOKEN_PRIVILEGES {
                    PrivilegeCount: 1,
                    Privileges: [LUID_AND_ATTRIBUTES {
                        Luid: luid,
                        Attributes: SE_PRIVILEGE_ENABLED,
                    }],
                };
                AdjustTokenPrivileges(token, false, Some(&privileges), 0, None, None)
            })
            .map_err(|e| e.message());
        // AdjustTokenPrivileges succeeds even when the privilege is missing.
        let missing = GetLastError() == ERROR_NOT_ALL_ASSIGNED;
        let _ = CloseHandle(token);
        result?;
        if missing {
            return Err("sem permissão para desligar este PC".into());
        }
        Ok(())
    }
}

/// S1–S3 through SetSuspendState; on Modern Standby PCs that refuse it, the display goes off.
fn suspend(hibernate: bool) -> Result<(), String> {
    enable_shutdown_privilege()?;
    if unsafe { SetSuspendState(hibernate, false, false) } {
        return Ok(());
    }
    let error = windows::core::Error::from_thread();
    if !hibernate && capabilities().is_some_and(|c| c.AoAc) {
        unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SYSCOMMAND,
                WPARAM(SC_MONITORPOWER as usize),
                LPARAM(2),
                SMTO_ABORTIFHUNG,
                1000,
                None,
            );
        }
        return Ok(());
    }
    Err(error.message())
}

/// "Inicialização Rápida": a normal shutdown from Start is hybrid when it is on.
fn hiberboot_enabled() -> bool {
    windows_registry::LOCAL_MACHINE
        .open("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Power")
        .and_then(|key| key.get_u32("HiberbootEnabled"))
        .is_ok_and(|value| value == 1)
}

pub(crate) fn shutdown_flags(restart: bool, hybrid: bool) -> SHUTDOWN_FLAGS {
    match (restart, hybrid) {
        (true, _) => SHUTDOWN_FLAGS(SHUTDOWN_RESTART.0 | SHUTDOWN_RESTARTAPPS.0),
        (false, true) => SHUTDOWN_FLAGS(SHUTDOWN_POWEROFF.0 | SHUTDOWN_HYBRID.0),
        (false, false) => SHUTDOWN_POWEROFF,
    }
}

/// Planned, not forced, no grace period: Windows still asks programs with unsaved work.
fn shutdown(restart: bool) -> Result<(), String> {
    enable_shutdown_privilege()?;
    let reason = SHUTDOWN_REASON(
        SHTDN_REASON_MAJOR_OTHER.0 | SHTDN_REASON_MINOR_OTHER.0 | SHTDN_REASON_FLAG_PLANNED.0,
    );
    let initiate =
        |flags| unsafe { InitiateShutdownW(PCWSTR::null(), PCWSTR::null(), 0, flags, reason) };
    let hybrid = !restart && hiberboot_enabled();
    let mut code = initiate(shutdown_flags(restart, hybrid));
    if code != 0 && hybrid {
        code = initiate(shutdown_flags(restart, false));
    }
    if code == 0 {
        Ok(())
    } else {
        Err(windows::core::Error::from_hresult(windows::core::HRESULT::from_win32(code)).message())
    }
}

/// Number of holders and the request handle (as an integer, so the static is Send).
static REQUEST: Mutex<(usize, isize)> = Mutex::new((0, 0));

struct WindowsAwake;

fn create_wake_lock() -> Option<HANDLE> {
    let mut reason: Vec<u16> = "PC Remote: transferindo arquivo com o celular"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let context = REASON_CONTEXT {
        Version: 0, // POWER_REQUEST_CONTEXT_VERSION
        Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
        Reason: REASON_CONTEXT_0 {
            SimpleReasonString: PWSTR(reason.as_mut_ptr()),
        },
    };
    unsafe {
        let handle = PowerCreateRequest(&context).ok()?;
        if PowerSetRequest(handle, PowerRequestSystemRequired).is_err() {
            let _ = CloseHandle(handle);
            return None;
        }
        Some(handle)
    }
}

impl Drop for WindowsAwake {
    fn drop(&mut self) {
        let mut state = REQUEST.lock().unwrap();
        state.0 -= 1;
        if state.0 == 0 && state.1 != 0 {
            let handle = HANDLE(state.1 as *mut _);
            unsafe {
                let _ = PowerClearRequest(handle, PowerRequestSystemRequired);
                let _ = CloseHandle(handle);
            }
            state.1 = 0;
        }
    }
}

impl PowerBackend for WindowsPower {
    fn sleep_supported(&self) -> bool {
        capabilities().is_some_and(|c| c.SystemS1 || c.SystemS2 || c.SystemS3 || c.AoAc)
    }

    fn lock_screen(&self) -> Result<(), PlatformError> {
        unsafe { LockWorkStation() }.map_err(|e| PlatformError::Unavailable(e.message()))
    }

    fn sleep(&self) -> Result<(), PlatformError> {
        suspend(false).map_err(PlatformError::Unavailable)
    }

    fn hibernate(&self) -> Result<(), PlatformError> {
        suspend(true).map_err(PlatformError::Unavailable)
    }

    fn shutdown(&self) -> Result<(), PlatformError> {
        shutdown(false).map_err(PlatformError::Unavailable)
    }

    fn reboot(&self) -> Result<(), PlatformError> {
        shutdown(true).map_err(PlatformError::Unavailable)
    }

    fn acquire_wake_lock(&self) -> Box<dyn Send> {
        let mut state = REQUEST.lock().unwrap();
        if state.0 == 0 {
            state.1 = create_wake_lock().map_or(0, |handle| handle.0 as isize);
        }
        state.0 += 1;
        Box::new(WindowsAwake)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_never_force_and_restart_brings_apps_back() {
        assert_eq!(shutdown_flags(false, false), SHUTDOWN_POWEROFF);
        assert_eq!(
            shutdown_flags(false, true).0,
            SHUTDOWN_POWEROFF.0 | SHUTDOWN_HYBRID.0
        );
        assert_eq!(
            shutdown_flags(true, true).0,
            SHUTDOWN_RESTART.0 | SHUTDOWN_RESTARTAPPS.0
        );
        // SHUTDOWN_FORCE_OTHERS (1) and SHUTDOWN_FORCE_SELF (2) are never set.
        for flags in [
            shutdown_flags(false, false),
            shutdown_flags(false, true),
            shutdown_flags(true, false),
        ] {
            assert_eq!(flags.0 & 0b11, 0);
        }
    }

    /// Read-only queries: nothing is locked, suspended or shut down.
    #[test]
    fn capability_query_and_privilege_are_harmless() {
        let _ = WindowsPower.sleep_supported();
        assert!(enable_shutdown_privilege().is_ok());
    }

    #[test]
    fn nested_holders_share_one_request() {
        let first = WindowsPower.acquire_wake_lock();
        let base = REQUEST.lock().unwrap().0;
        let handle = REQUEST.lock().unwrap().1;
        assert_ne!(handle, 0, "PowerCreateRequest failed");
        let second = WindowsPower.acquire_wake_lock();
        assert_eq!(*REQUEST.lock().unwrap(), (base + 1, handle));
        drop(second);
        assert_eq!(REQUEST.lock().unwrap().0, base);
        drop(first);
        let after = *REQUEST.lock().unwrap();
        assert!(after.0 > 0 || after.1 == 0);
    }
}
