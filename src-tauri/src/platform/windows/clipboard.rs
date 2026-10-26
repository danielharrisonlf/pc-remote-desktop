//! Windows clipboard implementation (`OpenClipboard`, `CF_UNICODETEXT`, `CF_DIBV5`, `"PNG"`).
use crate::{
    platform::{check_write_sequence, ClipboardBackend, PlatformError},
    protocol::{MAX_IMAGE_BYTES, MAX_TEXT},
    wic::{ClipImage, MAX_DIB_BYTES},
};
use std::{
    sync::{
        atomic::{AtomicIsize, Ordering},
        mpsc::Sender,
    },
    thread,
    time::Duration,
};
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND},
        System::{
            DataExchange::{
                AddClipboardFormatListener, CloseClipboard, EmptyClipboard, GetClipboardData,
                GetClipboardSequenceNumber, IsClipboardFormatAvailable, OpenClipboard,
                RegisterClipboardFormatW, SetClipboardData,
            },
            Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE},
            Ole::{CF_DIB, CF_DIBV5, CF_UNICODETEXT},
        },
        UI::WindowsAndMessaging::{
            CreateWindowExW, DispatchMessageW, GetMessageW, TranslateMessage, HWND_MESSAGE, MSG,
            WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLIPBOARDUPDATE,
        },
    },
};

pub struct WindowsClipboard;

static OWNER: AtomicIsize = AtomicIsize::new(0);

fn sequence() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}

fn open_retry(write: bool) -> Result<(), PlatformError> {
    let owner = if write {
        let raw = OWNER.load(Ordering::SeqCst);
        if raw == 0 {
            return Err(PlatformError::ClipboardBusy);
        }
        Some(HWND(raw as _))
    } else {
        None
    };
    for _ in 0..6 {
        if unsafe { OpenClipboard(owner) }.is_ok() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(25));
    }
    Err(PlatformError::ClipboardBusy)
}

struct Opened;
impl Drop for Opened {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

fn sensitive() -> bool {
    unsafe {
        let exclude = RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"));
        let cloud = RegisterClipboardFormatW(w!("CanUploadToCloudClipboard"));
        if exclude != 0 && IsClipboardFormatAvailable(exclude).is_ok() {
            return true;
        }
        if cloud != 0 && IsClipboardFormatAvailable(cloud).is_ok() {
            if let Ok(handle) = GetClipboardData(cloud) {
                let global = HGLOBAL(handle.0);
                if GlobalSize(global) >= 4 {
                    let ptr = GlobalLock(global) as *const u32;
                    if !ptr.is_null() {
                        let value = *ptr;
                        let _ = GlobalUnlock(global);
                        if value == 0 {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}

/// Closes the clipboard and returns the sequence the listener will report. CloseClipboard
/// adds the synthesized formats (CF_TEXT, CF_LOCALE…) and advances the sequence again, so a
/// value read while open would make our own write look like a new copy in Windows.
fn closed_sequence(opened: Opened) -> u32 {
    drop(opened);
    sequence()
}

/// The registered "PNG" format that browsers, Office and the Snipping Tool use.
fn png_format() -> u32 {
    unsafe { RegisterClipboardFormatW(w!("PNG")) }
}

/// Copies a clipboard format; the clipboard must be open. `too_large` above `max` bytes.
unsafe fn format_bytes(format: u32, max: usize) -> Result<Option<Vec<u8>>, PlatformError> {
    let Ok(handle) = (unsafe { GetClipboardData(format) }) else {
        return Ok(None);
    };
    let global = HGLOBAL(handle.0);
    let size = unsafe { GlobalSize(global) };
    if size == 0 {
        return Ok(None);
    }
    if size > max {
        return Err(PlatformError::TooLarge);
    }
    let ptr = unsafe { GlobalLock(global) } as *const u8;
    if ptr.is_null() {
        return Err(PlatformError::ClipboardBusy);
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr, size) }.to_vec();
    let _ = unsafe { GlobalUnlock(global) };
    Ok(Some(bytes))
}

/// Global memory handed to the clipboard; freed if the clipboard never takes it.
struct Owned(HGLOBAL);

impl Owned {
    fn copy(bytes: &[u8]) -> Result<Self, PlatformError> {
        unsafe {
            let global = GlobalAlloc(GMEM_MOVEABLE, bytes.len())
                .map_err(|e| PlatformError::Internal(e.to_string()))?;
            let owned = Self(global);
            let ptr = GlobalLock(global) as *mut u8;
            if ptr.is_null() {
                return Err(PlatformError::ClipboardBusy);
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
            let _ = GlobalUnlock(global);
            Ok(owned)
        }
    }
    /// SetClipboardData took it; the system frees it from now on.
    fn handed_over(self) {
        std::mem::forget(self);
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        unsafe {
            let _ = GlobalFree(Some(self.0));
        }
    }
}

impl ClipboardBackend for WindowsClipboard {
    fn sequence(&self) -> u32 {
        sequence()
    }

    fn read_text(&self) -> Result<Option<String>, PlatformError> {
        open_retry(false)?;
        let _opened = Opened;
        unsafe {
            if sensitive() {
                return Err(PlatformError::Sensitive);
            }
            if IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_err() {
                return Ok(None);
            }
            let handle = GetClipboardData(CF_UNICODETEXT.0 as u32)
                .map_err(|_| PlatformError::ClipboardBusy)?;
            let global = HGLOBAL(handle.0);
            let max_units = GlobalSize(global) / 2;
            if max_units > MAX_TEXT + 8 {
                return Err(PlatformError::TooLarge);
            }
            let ptr = GlobalLock(global) as *const u16;
            if ptr.is_null() {
                return Err(PlatformError::ClipboardBusy);
            }
            let units = std::slice::from_raw_parts(ptr, max_units);
            let end = units.iter().position(|v| *v == 0).unwrap_or(max_units);
            let decoded = String::from_utf16(&units[..end]);
            let _ = GlobalUnlock(global);
            let text = decoded.map_err(|_| PlatformError::InvalidMessage)?;
            if text.as_bytes().len() > MAX_TEXT {
                return Err(PlatformError::TooLarge);
            }
            Ok(Some(text))
        }
    }

    fn write_text(&self, text: &str, expected_sequence: Option<u32>) -> Result<u32, PlatformError> {
        if text.as_bytes().len() > MAX_TEXT {
            return Err(PlatformError::TooLarge);
        }
        if text.contains('\0') {
            return Err(PlatformError::InvalidMessage);
        }
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        open_retry(true)?;
        let opened = Opened;
        // OpenClipboard excludes other writers. Check here, before allocating or
        // emptying, so a copy whose listener is still pending cannot be overwritten.
        check_write_sequence(expected_sequence, sequence())?;
        unsafe {
            let global = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2)
                .map_err(|e| PlatformError::Internal(e.to_string()))?;
            let ptr = GlobalLock(global) as *mut u16;
            if ptr.is_null() {
                let _ = GlobalFree(Some(global));
                return Err(PlatformError::ClipboardBusy);
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
            let _ = GlobalUnlock(global);
            if let Err(e) = EmptyClipboard() {
                let _ = GlobalFree(Some(global));
                return Err(PlatformError::Internal(e.to_string()));
            }
            if SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(global.0))).is_err() {
                let _ = GlobalFree(Some(global));
                return Err(PlatformError::ClipboardBusy);
            }
        }
        Ok(closed_sequence(opened))
    }

    fn read_image(&self) -> Result<Option<ClipImage>, PlatformError> {
        open_retry(false)?;
        let _opened = Opened;
        unsafe {
            if sensitive() {
                return Err(PlatformError::Sensitive);
            }
            if IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_ok() {
                return Ok(None);
            }
            let png = png_format();
            if png != 0 && IsClipboardFormatAvailable(png).is_ok() {
                if let Some(bytes) = format_bytes(png, MAX_IMAGE_BYTES as usize)? {
                    return Ok(Some(ClipImage::Png(bytes)));
                }
            }
            // CF_DIB first: a DIBV5 that Windows synthesizes from it may carry a zero alpha.
            for format in [CF_DIB.0 as u32, CF_DIBV5.0 as u32] {
                if IsClipboardFormatAvailable(format).is_ok() {
                    if let Some(bytes) = format_bytes(format, MAX_DIB_BYTES)? {
                        return Ok(Some(ClipImage::Dib(bytes)));
                    }
                }
            }
            Ok(None)
        }
    }

    fn write_image(
        &self,
        png: &[u8],
        dibv5: &[u8],
        expected_sequence: Option<u32>,
    ) -> Result<u32, PlatformError> {
        let png_format = png_format();
        if png_format == 0 {
            return Err(PlatformError::ClipboardBusy);
        }
        let dib = Owned::copy(dibv5)?;
        let png = Owned::copy(png)?;
        open_retry(true)?;
        let opened = Opened;
        check_write_sequence(expected_sequence, sequence())?;
        unsafe {
            EmptyClipboard().map_err(|e| PlatformError::Internal(e.to_string()))?;
            if SetClipboardData(CF_DIBV5.0 as u32, Some(HANDLE(dib.0 .0))).is_err() {
                return Err(PlatformError::ClipboardBusy);
            }
            dib.handed_over();
            if SetClipboardData(png_format, Some(HANDLE(png.0 .0))).is_err() {
                return Err(PlatformError::ClipboardBusy);
            }
            png.handed_over();
        }
        Ok(closed_sequence(opened))
    }

    fn start_listener(&self, tx: Sender<u32>) {
        thread::spawn(move || unsafe {
            let hwnd = match CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                PCWSTR::null(),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            ) {
                Ok(h) => h,
                Err(_) => return,
            };
            OWNER.store(hwnd.0 as isize, Ordering::SeqCst);
            // Without the listener, keep the window alive anyway: writes open the
            // clipboard with it, and returning would leave OWNER pointing at a
            // destroyed window so every write failed until restart.
            let _ = AddClipboardFormatListener(hwnd);
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                if msg.message == WM_CLIPBOARDUPDATE {
                    let _ = tx.send(GetClipboardSequenceNumber());
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "modifica temporariamente o clipboard real do Windows"]
    fn windows_unicode_clipboard_smoke() {
        let original = match WindowsClipboard.read_text() {
            Ok(Some(text)) => text,
            _ => {
                eprintln!(
                    "SKIP: current clipboard is not readable plain text; preserved without changes"
                );
                return;
            }
        };
        let (tx, _rx) = std::sync::mpsc::channel();
        WindowsClipboard.start_listener(tx);
        for _ in 0..50 {
            if OWNER.load(Ordering::SeqCst) != 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        struct Restore(String);
        impl Drop for Restore {
            fn drop(&mut self) {
                let _ = WindowsClipboard.write_text(&self.0, None);
            }
        }
        let _restore = Restore(original);
        let sample = "Olá 👋\nhttps://example.org\n漢字";
        let written_sequence = WindowsClipboard.write_text(sample, None).unwrap();
        assert_eq!(written_sequence, WindowsClipboard.sequence());
        assert_eq!(
            WindowsClipboard.write_text(
                "must not overwrite the current copy",
                Some(written_sequence.wrapping_sub(1))
            ),
            Err(PlatformError::Conflict)
        );
        assert_eq!(
            WindowsClipboard.read_text().unwrap().as_deref(),
            Some(sample)
        );
    }

    #[test]
    #[ignore = "modifica temporariamente o clipboard real do Windows"]
    fn windows_image_clipboard_smoke() {
        let original = match WindowsClipboard.read_text() {
            Ok(Some(text)) => text,
            _ => {
                eprintln!(
                    "SKIP: current clipboard is not readable plain text; preserved without changes"
                );
                return;
            }
        };
        let (tx, _rx) = std::sync::mpsc::channel();
        WindowsClipboard.start_listener(tx);
        for _ in 0..50 {
            if OWNER.load(Ordering::SeqCst) != 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        struct Restore(String);
        impl Drop for Restore {
            fn drop(&mut self) {
                let _ = WindowsClipboard.write_text(&self.0, None);
            }
        }
        let _restore = Restore(original);
        let png = crate::wic::tests::sample_png();
        let (kept, dib) = crate::wic::clipboard_formats(&png).unwrap();
        let written = WindowsClipboard.write_image(&kept, &dib, None).unwrap();
        assert_eq!(written, WindowsClipboard.sequence());
        assert_eq!(
            WindowsClipboard.write_image(&kept, &dib, Some(written.wrapping_sub(1))),
            Err(PlatformError::Conflict)
        );
        match WindowsClipboard.read_image().unwrap() {
            Some(ClipImage::Png(bytes)) => assert_eq!(bytes, kept),
            _ => panic!("PNG format missing"),
        }
        assert_eq!(WindowsClipboard.read_text().unwrap(), None);
        let converted = crate::wic::png_for_phone(ClipImage::Dib(dib.clone())).expect("DIB to PNG");
        assert!(crate::wic::is_png(&converted));
    }
}
