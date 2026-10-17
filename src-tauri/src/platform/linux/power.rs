//! Linux power & session management (`PowerBackend`) via `systemd-logind` D-Bus (`org.freedesktop.login1`).
//!
//! Uses `org.freedesktop.login1.Manager` on the system bus for:
//! - `LockSession` / `LockSessions` (with DE screensaver D-Bus and `loginctl` fallbacks)
//! - `Suspend(true)`
//! - `Hibernate(true)`
//! - `PowerOff(true)`
//! - `Reboot(true)`
//! - `CanSuspend`
//! - `Inhibit("sleep:idle", ...)` wake locks held during active file transfers.
use crate::platform::{PlatformError, PowerBackend};
use std::{process::Command, sync::Mutex};
use zbus::{
    blocking::{Connection, Proxy},
    zvariant::OwnedFd,
};

pub struct LinuxPower;

const LOGIN1_DEST: &str = "org.freedesktop.login1";
const LOGIN1_PATH: &str = "/org/freedesktop/login1";
const LOGIN1_MANAGER: &str = "org.freedesktop.login1.Manager";

/// Ensures blocking D-Bus calls never block a Tokio worker thread runtime.
fn run_blocking_safe<F, T>(f: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    if tokio::runtime::Handle::try_current().is_ok() {
        std::thread::spawn(f)
            .join()
            .unwrap_or_else(|_| panic!("blocking D-Bus thread panicked"))
    } else {
        f()
    }
}

fn call_login1<B, R>(method: &'static str, body: B) -> Result<R, String>
where
    B: serde::Serialize + zbus::zvariant::DynamicType + Send + 'static,
    R: serde::de::DeserializeOwned + zbus::zvariant::Type + Send + 'static,
{
    run_blocking_safe(move || {
        let conn =
            Connection::system().map_err(|e| format!("D-Bus do sistema indisponível: {e}"))?;
        let proxy = Proxy::new(&conn, LOGIN1_DEST, LOGIN1_PATH, LOGIN1_MANAGER)
            .map_err(|e| e.to_string())?;
        proxy.call(method, &body).map_err(|e| e.to_string())
    })
}

fn call_session_screensaver_lock() -> bool {
    run_blocking_safe(|| {
        let Ok(conn) = Connection::session() else {
            return false;
        };
        for (dest, path, iface) in [
            (
                "org.freedesktop.ScreenSaver",
                "/org/freedesktop/ScreenSaver",
                "org.freedesktop.ScreenSaver",
            ),
            (
                "org.gnome.ScreenSaver",
                "/org/gnome/ScreenSaver",
                "org.gnome.ScreenSaver",
            ),
        ] {
            if let Ok(proxy) = Proxy::new(&conn, dest, path, iface) {
                if proxy.call_method("Lock", &()).is_ok() {
                    return true;
                }
            }
        }
        false
    })
}

/// Reference count and the `systemd-logind` inhibitor file descriptor kept open while transfers run.
static INHIBITOR: Mutex<(usize, Option<OwnedFd>)> = Mutex::new((0, None));

struct LinuxAwake;

fn create_inhibitor() -> Option<OwnedFd> {
    call_login1::<_, OwnedFd>(
        "Inhibit",
        (
            "sleep:idle",
            "PC Remote",
            "PC Remote: transferindo arquivo com o celular",
            "block",
        ),
    )
    .ok()
}

impl Drop for LinuxAwake {
    fn drop(&mut self) {
        let mut state = INHIBITOR.lock().unwrap_or_else(|p| p.into_inner());
        state.0 = state.0.saturating_sub(1);
        if state.0 == 0 {
            // Dropping the OwnedFd closes the inhibitor pipe and releases the systemd wake lock.
            state.1 = None;
        }
    }
}

impl PowerBackend for LinuxPower {
    fn sleep_supported(&self) -> bool {
        if let Ok(reply) = call_login1::<_, String>("CanSuspend", ()) {
            let trimmed = reply.trim();
            if trimmed == "yes" || trimmed == "challenge" {
                return true;
            }
            if trimmed == "no" || trimmed == "na" {
                return false;
            }
        }
        std::fs::read_to_string("/sys/power/state").is_ok_and(|states| {
            states
                .split_whitespace()
                .any(|s| s == "mem" || s == "s2idle" || s == "standby")
        })
    }

    fn lock_screen(&self) -> Result<(), PlatformError> {
        // 1. Try systemd-logind LockSession on current session or LockSessions across sessions
        if let Ok(session_id) = std::env::var("XDG_SESSION_ID") {
            if !session_id.trim().is_empty()
                && call_login1::<_, ()>("LockSession", (session_id,)).is_ok()
            {
                return Ok(());
            }
        }
        if call_login1::<_, ()>("LockSessions", ()).is_ok() {
            return Ok(());
        }
        // 2. Fallback to GNOME / freedesktop ScreenSaver over session D-Bus
        if call_session_screensaver_lock() {
            return Ok(());
        }
        // 3. Fallback to loginctl / xdg-screensaver CLI
        if Command::new("loginctl")
            .arg("lock-session")
            .status()
            .is_ok_and(|s| s.success())
            || Command::new("xdg-screensaver")
                .arg("lock")
                .status()
                .is_ok_and(|s| s.success())
        {
            return Ok(());
        }
        tracing::warn!("Linux lock_screen failed across login1, ScreenSaver D-Bus, and loginctl");
        Err(PlatformError::Unavailable(
            "Não foi possível bloquear a sessão Linux".into(),
        ))
    }

    fn sleep(&self) -> Result<(), PlatformError> {
        call_login1::<_, ()>("Suspend", (true,)).map_err(|err| {
            tracing::warn!("systemd-logind Suspend(true) failed: {err}");
            PlatformError::Unavailable(err)
        })
    }

    fn hibernate(&self) -> Result<(), PlatformError> {
        call_login1::<_, ()>("Hibernate", (true,)).map_err(|err| {
            tracing::warn!("systemd-logind Hibernate(true) failed: {err}");
            PlatformError::Unavailable(err)
        })
    }

    fn shutdown(&self) -> Result<(), PlatformError> {
        call_login1::<_, ()>("PowerOff", (true,)).map_err(|err| {
            tracing::warn!("systemd-logind PowerOff(true) failed: {err}");
            PlatformError::Unavailable(err)
        })
    }

    fn reboot(&self) -> Result<(), PlatformError> {
        call_login1::<_, ()>("Reboot", (true,)).map_err(|err| {
            tracing::warn!("systemd-logind Reboot(true) failed: {err}");
            PlatformError::Unavailable(err)
        })
    }

    fn acquire_wake_lock(&self) -> Box<dyn Send> {
        let mut state = INHIBITOR.lock().unwrap_or_else(|p| p.into_inner());
        if state.0 == 0 {
            state.1 = create_inhibitor();
        }
        state.0 += 1;
        Box::new(LinuxAwake)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_query_and_wake_lock_refcount_do_not_panic() {
        let _ = LinuxPower.sleep_supported();
        let first = LinuxPower.acquire_wake_lock();
        let base = INHIBITOR.lock().unwrap_or_else(|p| p.into_inner()).0;
        assert!(base >= 1);
        let second = LinuxPower.acquire_wake_lock();
        assert_eq!(
            INHIBITOR.lock().unwrap_or_else(|p| p.into_inner()).0,
            base + 1
        );
        drop(second);
        assert_eq!(INHIBITOR.lock().unwrap_or_else(|p| p.into_inner()).0, base);
        drop(first);
    }
}
