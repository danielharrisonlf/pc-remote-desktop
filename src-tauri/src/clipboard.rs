//! System clipboard operations. Delegates to the active platform's `ClipboardBackend`.
use crate::wic::ClipImage;
use std::sync::mpsc::Sender;

pub fn sequence() -> u32 {
    crate::platform::clipboard().sequence()
}

pub fn read() -> Result<Option<String>, String> {
    crate::platform::clipboard()
        .read_text()
        .map_err(|e| e.into_legacy_message())
}

pub fn write_checked(text: &str, expected_sequence: Option<u32>) -> Result<u32, String> {
    crate::platform::clipboard()
        .write_text(text, expected_sequence)
        .map_err(|e| e.into_legacy_message())
}

/// clipboard.v2: the image of the clipboard when it holds no text, as copied by its owner.
/// Conversion happens after the clipboard is closed (see `wic::png_for_phone`).
pub fn read_image() -> Result<Option<ClipImage>, String> {
    crate::platform::clipboard()
        .read_image()
        .map_err(|e| e.into_legacy_message())
}

/// clipboard.v2: writes the image to the system clipboard. Like `write_checked`, an automatic
/// write needs the sequence it was based on.
pub fn write_image_checked(
    png: &[u8],
    dibv5: &[u8],
    expected_sequence: Option<u32>,
) -> Result<u32, String> {
    crate::platform::clipboard()
        .write_image(png, dibv5, expected_sequence)
        .map_err(|e| e.into_legacy_message())
}

pub fn start_listener(tx: Sender<u32>) {
    crate::platform::clipboard().start_listener(tx);
}

#[cfg(test)]
mod tests {
    use crate::platform::{check_write_sequence, PlatformError};

    #[test]
    fn automatic_write_rejects_a_copy_waiting_for_its_listener() {
        assert_eq!(
            check_write_sequence(Some(41), 42),
            Err(PlatformError::Conflict)
        );
        assert_eq!(check_write_sequence(Some(41), 41), Ok(()));
        assert_eq!(check_write_sequence(None, 42), Ok(()));
        // Sequence values wrap; equality, rather than ordering, is required.
        assert_eq!(
            check_write_sequence(Some(u32::MAX), 0),
            Err(PlatformError::Conflict)
        );
    }
}
