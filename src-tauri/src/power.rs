//! Keeps Windows from sleeping while a file transfer runs in either direction.
//!
//! A power request is per process and not tied to a thread, unlike
//! SetThreadExecutionState, so tokio tasks may hold it. `powercfg /requests`
//! lists it with the reason below while a transfer is active.
use std::sync::Mutex;
use windows::{
    core::PWSTR,
    Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::{
            Power::{
                PowerClearRequest, PowerCreateRequest, PowerRequestSystemRequired, PowerSetRequest,
            },
            Threading::{POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0},
        },
    },
};

/// Number of holders and the request handle (as an integer, so the static is Send).
static REQUEST: Mutex<(usize, isize)> = Mutex::new((0, 0));

/// Held for the duration of one transfer.
pub struct Awake(());

pub fn awake() -> Awake {
    let mut state = REQUEST.lock().unwrap();
    if state.0 == 0 {
        state.1 = create().map_or(0, |handle| handle.0 as isize);
    }
    state.0 += 1;
    Awake(())
}

fn create() -> Option<HANDLE> {
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

impl Drop for Awake {
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

#[cfg(test)]
mod tests {
    #[test]
    fn nested_holders_share_one_request() {
        // Transfer tests may hold the request at the same time; count relative to them.
        let first = super::awake();
        let base = super::REQUEST.lock().unwrap().0;
        let handle = super::REQUEST.lock().unwrap().1;
        assert_ne!(handle, 0, "PowerCreateRequest failed");
        let second = super::awake();
        assert_eq!(*super::REQUEST.lock().unwrap(), (base + 1, handle));
        drop(second);
        assert_eq!(super::REQUEST.lock().unwrap().0, base);
        drop(first);
        let after = *super::REQUEST.lock().unwrap();
        assert!(after.0 > 0 || after.1 == 0);
    }
}
