//! Native "Enviar ao celular" picker. Paths stay in Rust; the webview never reads files.
//! Delegates to the active platform's `SystemBackend::pick_files`.
use std::path::PathBuf;

/// Shows the native file picker and returns the chosen files; empty when cancelled.
pub fn pick_files(owner: isize) -> Result<Vec<PathBuf>, String> {
    crate::platform::system().pick_files(owner)
}
