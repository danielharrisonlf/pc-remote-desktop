//! System master volume and mute control. Delegates to the active platform's `AudioBackend`.
use crate::protocol::AudioState;

pub fn read() -> AudioState {
    crate::platform::audio().read()
}

pub fn set(
    percent: Option<u8>,
    step: Option<i8>,
    muted: Option<bool>,
) -> Result<AudioState, String> {
    crate::platform::audio()
        .set(percent, step, muted)
        .map_err(|e| e.into_legacy_message())
}
