//! Windows Core Audio (`IAudioEndpointVolume`) implementation of `AudioBackend`.
use crate::{
    platform::{AudioBackend, PlatformError},
    protocol::AudioState,
};
use windows::Win32::{
    Media::Audio::{
        eConsole, eRender, Endpoints::IAudioEndpointVolume, IMMDeviceEnumerator, MMDeviceEnumerator,
    },
    System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    },
};

pub struct WindowsAudio;

static AUDIO_WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn endpoint() -> windows::core::Result<IAudioEndpointVolume> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
        device.Activate(CLSCTX_ALL, None)
    }
}

pub fn with_com<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(|e| e.to_string())?;
        let result = f();
        CoUninitialize();
        result
    }
}

impl AudioBackend for WindowsAudio {
    fn read(&self) -> AudioState {
        match with_com(|| {
            let e = endpoint().map_err(|e| e.to_string())?;
            unsafe {
                Ok((
                    (e.GetMasterVolumeLevelScalar().map_err(|e| e.to_string())? * 100.0)
                        .round()
                        .clamp(0.0, 100.0) as u8,
                    e.GetMute().map_err(|e| e.to_string())?.as_bool(),
                ))
            }
        }) {
            Ok((percent, muted)) => AudioState {
                available: true,
                percent,
                muted,
                error: None,
            },
            Err(_) => AudioState {
                available: false,
                percent: 0,
                muted: false,
                error: Some("Dispositivo de saída indisponível".into()),
            },
        }
    }

    fn set(
        &self,
        percent: Option<u8>,
        step: Option<i8>,
        muted: Option<bool>,
    ) -> Result<AudioState, PlatformError> {
        let _guard = AUDIO_WRITE
            .lock()
            .map_err(|_| PlatformError::Internal("Audio mutex poisoned".to_string()))?;
        with_com(|| {
            let e = endpoint().map_err(|_| "Dispositivo de saída indisponível".to_string())?;
            unsafe {
                if let Some(p) = percent {
                    e.SetMasterVolumeLevelScalar(p as f32 / 100.0, std::ptr::null())
                        .map_err(|e| e.to_string())?;
                }
                if let Some(d) = step {
                    let p = e.GetMasterVolumeLevelScalar().map_err(|e| e.to_string())?;
                    e.SetMasterVolumeLevelScalar(
                        (p + d as f32 / 100.0).clamp(0.0, 1.0),
                        std::ptr::null(),
                    )
                    .map_err(|e| e.to_string())?;
                }
                if let Some(m) = muted {
                    e.SetMute(m, std::ptr::null()).map_err(|e| e.to_string())?;
                }
                Ok(AudioState {
                    available: true,
                    percent: (e.GetMasterVolumeLevelScalar().map_err(|e| e.to_string())? * 100.0)
                        .round()
                        .clamp(0.0, 100.0) as u8,
                    muted: e.GetMute().map_err(|e| e.to_string())?.as_bool(),
                    error: None,
                })
            }
        })
        .map_err(PlatformError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "modifica temporariamente o volume real do Windows"]
    fn windows_audio_smoke() {
        let initial = WindowsAudio.read();
        if !initial.available {
            eprintln!("SKIP: no available Windows audio endpoint");
            return;
        }
        struct Restore(AudioState);
        impl Drop for Restore {
            fn drop(&mut self) {
                let _ = WindowsAudio.set(Some(self.0.percent), None, Some(self.0.muted));
            }
        }
        let _restore = Restore(initial.clone());
        let target = if initial.percent < 50 {
            initial.percent + 1
        } else {
            initial.percent - 1
        };
        let changed = WindowsAudio.set(Some(target), None, None).unwrap();
        assert_eq!(changed.percent, target);
        let muted = WindowsAudio.set(None, None, Some(!initial.muted)).unwrap();
        assert_eq!(muted.muted, !initial.muted);
    }
}
