//! Linux clipboard backend (`ClipboardBackend`) using `arboard` and `image`.
//!
//! Supports reading and writing UTF-8 text and RGBA/PNG images across X11 and Wayland,
//! maintaining a monotonic sequence number and a background change-detection listener.
use crate::{
    platform::{check_write_sequence, ClipboardBackend, PlatformError},
    protocol::{MAX_IMAGE_BYTES, MAX_TEXT},
    wic::ClipImage,
};
use arboard::{Clipboard, ImageData};
use image::{ImageFormat, RgbaImage};
use sha2::{Digest, Sha256};
use std::{
    borrow::Cow,
    io::Cursor,
    sync::{
        atomic::{AtomicU32, Ordering},
        mpsc::Sender,
        Mutex, OnceLock,
    },
    thread,
    time::Duration,
};

pub struct LinuxClipboard;

static SEQUENCE: AtomicU32 = AtomicU32::new(1);

struct ClipGuard {
    clipboard: Option<Clipboard>,
    last_digest: Option<[u8; 32]>,
}

static STATE: OnceLock<Mutex<ClipGuard>> = OnceLock::new();

fn state() -> &'static Mutex<ClipGuard> {
    STATE.get_or_init(|| {
        Mutex::new(ClipGuard {
            clipboard: Clipboard::new().ok(),
            last_digest: None,
        })
    })
}

fn with_clipboard<T>(
    f: impl FnOnce(&mut Clipboard, &mut Option<[u8; 32]>) -> Result<T, PlatformError>,
) -> Result<T, PlatformError> {
    let mut guard = state().lock().map_err(|_| PlatformError::ClipboardBusy)?;
    if guard.clipboard.is_none() {
        guard.clipboard = Clipboard::new().ok();
    }
    let ClipGuard {
        clipboard,
        last_digest,
    } = &mut *guard;
    let Some(cb) = clipboard.as_mut() else {
        return Err(PlatformError::ClipboardBusy);
    };
    f(cb, last_digest)
}

fn hash_bytes(tag: u8, bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update([tag]);
    hasher.update(bytes);
    hasher.finalize().into()
}

fn bump_sequence(last_digest: &mut Option<[u8; 32]>, new_digest: [u8; 32]) -> u32 {
    if *last_digest != Some(new_digest) {
        *last_digest = Some(new_digest);
        SEQUENCE.fetch_add(1, Ordering::SeqCst).wrapping_add(1)
    } else {
        SEQUENCE.load(Ordering::SeqCst)
    }
}

impl ClipboardBackend for LinuxClipboard {
    fn sequence(&self) -> u32 {
        SEQUENCE.load(Ordering::SeqCst)
    }

    fn read_text(&self) -> Result<Option<String>, PlatformError> {
        with_clipboard(|cb, _| match cb.get_text() {
            Ok(text) => {
                if text.as_bytes().len() > MAX_TEXT {
                    return Err(PlatformError::TooLarge);
                }
                if text.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(text))
                }
            }
            Err(arboard::Error::ContentNotAvailable) => Ok(None),
            Err(err) => {
                tracing::debug!("Linux clipboard get_text error: {err}");
                Ok(None)
            }
        })
    }

    fn write_text(&self, text: &str, expected_sequence: Option<u32>) -> Result<u32, PlatformError> {
        if text.as_bytes().len() > MAX_TEXT {
            return Err(PlatformError::TooLarge);
        }
        if text.contains('\0') {
            return Err(PlatformError::InvalidMessage);
        }
        with_clipboard(|cb, last_digest| {
            check_write_sequence(expected_sequence, SEQUENCE.load(Ordering::SeqCst))?;
            cb.set_text(text.to_owned())
                .map_err(|_| PlatformError::ClipboardBusy)?;
            let digest = hash_bytes(b'T', text.as_bytes());
            Ok(bump_sequence(last_digest, digest))
        })
    }

    fn read_image(&self) -> Result<Option<ClipImage>, PlatformError> {
        with_clipboard(|cb, _| {
            // Text takes precedence over images, matching the Windows contract.
            if cb.get_text().is_ok_and(|t| !t.is_empty()) {
                return Ok(None);
            }
            let img = match cb.get_image() {
                Ok(img) => img,
                Err(arboard::Error::ContentNotAvailable) => return Ok(None),
                Err(_) => return Ok(None),
            };
            if img.width == 0 || img.height == 0 {
                return Ok(None);
            }
            let rgba =
                RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned())
                    .ok_or(PlatformError::ClipboardBusy)?;
            let mut png = Vec::new();
            rgba.write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
                .map_err(|e| PlatformError::Internal(e.to_string()))?;
            if png.len() as u64 > MAX_IMAGE_BYTES {
                return Err(PlatformError::TooLarge);
            }
            Ok(Some(ClipImage::Png(png)))
        })
    }

    fn write_image(
        &self,
        png: &[u8],
        _dibv5: &[u8],
        expected_sequence: Option<u32>,
    ) -> Result<u32, PlatformError> {
        let decoded = image::load_from_memory_with_format(png, ImageFormat::Png)
            .map_err(|_| PlatformError::InvalidMessage)?
            .into_rgba8();
        let width = decoded.width() as usize;
        let height = decoded.height() as usize;
        let raw = decoded.into_raw();

        with_clipboard(|cb, last_digest| {
            check_write_sequence(expected_sequence, SEQUENCE.load(Ordering::SeqCst))?;
            let data = ImageData {
                width,
                height,
                bytes: Cow::Owned(raw),
            };
            cb.set_image(data)
                .map_err(|_| PlatformError::ClipboardBusy)?;
            let digest = hash_bytes(b'I', png);
            Ok(bump_sequence(last_digest, digest))
        })
    }

    fn start_listener(&self, tx: Sender<u32>) {
        thread::Builder::new()
            .name("pc-remote-clipboard-linux".into())
            .spawn(move || loop {
                thread::sleep(Duration::from_millis(450));
                let changed_seq = with_clipboard(|cb, last_digest| {
                    if let Ok(text) = cb.get_text() {
                        if !text.is_empty() && text.as_bytes().len() <= MAX_TEXT {
                            let digest = hash_bytes(b'T', text.as_bytes());
                            if *last_digest != Some(digest) {
                                return Ok(Some(bump_sequence(last_digest, digest)));
                            }
                            return Ok(None);
                        }
                    }
                    if let Ok(img) = cb.get_image() {
                        if img.width > 0 && img.height > 0 {
                            let digest = hash_bytes(b'I', &img.bytes);
                            if *last_digest != Some(digest) {
                                return Ok(Some(bump_sequence(last_digest, digest)));
                            }
                        }
                    }
                    Ok(None)
                })
                .ok()
                .flatten();

                if let Some(seq) = changed_seq {
                    if tx.send(seq).is_err() {
                        break;
                    }
                }
            })
            .ok();
    }
}
