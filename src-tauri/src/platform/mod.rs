//! Cross-platform abstraction layer (Ports & Adapters).
//!
//! All operating-system specific implementations (Windows Win32/WinRT and Linux X11/Wayland/D-Bus/XDG)
//! live strictly under `platform::windows` and `platform::linux`. Business logic modules interact only
//! with these traits and the compile-time factory functions exposed here.

#[cfg(windows)]
pub mod windows;

#[cfg(target_os = "linux")]
pub mod linux;

use crate::{
    energy::PowerSystem,
    files::SafeName,
    protocol::{
        Action, AudioState, KeyboardKey, MediaCommand, MediaState, PointerButton, PowerAction,
    },
    wic::ClipImage,
};
use std::{
    fmt,
    path::{Path, PathBuf},
    sync::mpsc::Sender,
};
use uuid::Uuid;

/// Unified error type returned by platform backends.
/// Never panics on missing daemons (D-Bus, PipeWire, X11/Wayland); degrades gracefully.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformError {
    InvalidMessage,
    Unavailable(String),
    Unauthorized,
    Conflict,
    Sensitive,
    TooLarge,
    ClipboardBusy,
    Io(String),
    Internal(String),
}

impl PlatformError {
    /// Maps platform error to the wire protocol code where applicable.
    pub fn protocol_code(&self) -> &'static str {
        match self {
            Self::InvalidMessage => "invalid_message",
            Self::Unavailable(_) => "unavailable",
            Self::Unauthorized => "unauthorized",
            Self::Conflict => "conflict",
            Self::Sensitive => "sensitive",
            Self::TooLarge => "too_large",
            Self::ClipboardBusy => "clipboard_busy",
            Self::Io(_) | Self::Internal(_) => "internal",
        }
    }

    /// Maps platform error to the legacy string format expected by callers.
    pub fn into_legacy_message(self) -> String {
        match self {
            Self::Unavailable(msg) if !msg.is_empty() => msg,
            Self::Io(msg) if !msg.is_empty() => msg,
            Self::Internal(msg) if !msg.is_empty() => msg,
            other => other.protocol_code().to_string(),
        }
    }
}

impl fmt::Display for PlatformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMessage => write!(f, "invalid_message"),
            Self::Unavailable(msg) if !msg.is_empty() => write!(f, "{msg}"),
            Self::Unavailable(_) => write!(f, "unavailable"),
            Self::Unauthorized => write!(f, "unauthorized"),
            Self::Conflict => write!(f, "conflict"),
            Self::Sensitive => write!(f, "sensitive"),
            Self::TooLarge => write!(f, "too_large"),
            Self::ClipboardBusy => write!(f, "clipboard_busy"),
            Self::Io(msg) => write!(f, "{msg}"),
            Self::Internal(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for PlatformError {}

/// Mouse and keyboard input simulation backend.
pub trait InputBackend: Send + Sync {
    fn move_pointer(&self, dx: i16, dy: i16) -> Result<(), PlatformError>;
    fn click_pointer(&self, button: PointerButton, count: u8) -> Result<(), PlatformError>;
    fn scroll_pointer(&self, steps: i8) -> Result<(), PlatformError>;
    fn key_down(&self, key: KeyboardKey) -> Result<(), PlatformError>;
    fn key_up(&self, key: KeyboardKey) -> Result<(), PlatformError>;
    fn press_key(&self, key: KeyboardKey) -> Result<(), PlatformError> {
        self.key_down(key)?;
        self.key_up(key)
    }
    fn type_unicode(&self, text: &str) -> Result<(), PlatformError>;

    /// Executes a high-level protocol input action.
    fn execute(&self, action: &Action) -> Result<(), PlatformError> {
        match action {
            Action::PointerMove { dx, dy } => self.move_pointer(*dx, *dy),
            Action::PointerClick { button, count } => self.click_pointer(*button, *count),
            Action::PointerScroll { steps } => self.scroll_pointer(*steps),
            Action::KeyboardText { text } => self.type_unicode(text),
            Action::KeyboardKey { key } => self.press_key(*key),
            _ => Err(PlatformError::InvalidMessage),
        }
    }
}

/// Master audio volume and mute backend.
pub trait AudioBackend: Send + Sync {
    fn read(&self) -> AudioState;
    fn set(
        &self,
        percent: Option<u8>,
        step: Option<i8>,
        muted: Option<bool>,
    ) -> Result<AudioState, PlatformError>;
}

/// System media transport controls backend (GSMTC on Windows, MPRIS D-Bus on Linux).
pub trait MediaBackend: Send + Sync {
    fn read(&self) -> MediaState;
    fn control(
        &self,
        id: &str,
        command: &MediaCommand,
        offset: Option<i8>,
        permit: Box<dyn FnOnce() -> bool + Send>,
    ) -> Result<(), PlatformError>;
}

/// Power and session management backend (Win32 Power/Shutdown on Windows, systemd-logind on Linux).
pub trait PowerBackend: Send + Sync {
    fn sleep_supported(&self) -> bool;
    fn lock_screen(&self) -> Result<(), PlatformError>;
    fn sleep(&self) -> Result<(), PlatformError>;
    fn hibernate(&self) -> Result<(), PlatformError>;
    fn shutdown(&self) -> Result<(), PlatformError>;
    fn reboot(&self) -> Result<(), PlatformError>;

    fn run_action(&self, action: PowerAction) -> Result<(), PlatformError> {
        match action {
            PowerAction::Lock => self.lock_screen(),
            PowerAction::Sleep => self.sleep(),
            PowerAction::Shutdown => self.shutdown(),
            PowerAction::Restart => self.reboot(),
        }
    }

    /// Acquires a wake lock / sleep inhibitor held for the duration of a file transfer.
    fn acquire_wake_lock(&self) -> Box<dyn Send>;
}

/// Adapter bridging `PowerBackend` to `energy::PowerSystem`.
pub struct PlatformPowerSystem;

impl PowerSystem for PlatformPowerSystem {
    fn sleep_supported(&self) -> bool {
        power().sleep_supported()
    }
    fn run(&self, action: PowerAction) -> Result<(), String> {
        power()
            .run_action(action)
            .map_err(PlatformError::into_legacy_message)
    }
}

/// Clipboard text and image backend.
pub trait ClipboardBackend: Send + Sync {
    fn sequence(&self) -> u32;
    fn read_text(&self) -> Result<Option<String>, PlatformError>;
    fn write_text(&self, text: &str, expected_sequence: Option<u32>) -> Result<u32, PlatformError>;
    fn read_image(&self) -> Result<Option<ClipImage>, PlatformError>;
    fn write_image(
        &self,
        png: &[u8],
        dibv5: &[u8],
        expected_sequence: Option<u32>,
    ) -> Result<u32, PlatformError>;
    fn start_listener(&self, tx: Sender<u32>);
}

/// Autostart configuration backend (Windows Registry on Windows, XDG `.desktop` on Linux).
pub trait AutostartBackend: Send + Sync {
    fn set_enabled(&self, enabled: bool) -> Result<(), PlatformError>;
}

/// OS-specific system, filesystem, secret protection, and shell operations.
pub trait SystemBackend: Send + Sync {
    fn os_name(&self) -> &'static str;
    fn hostname(&self) -> String;
    fn monotonic_ms(&self) -> u64;
    fn local_hms(&self) -> (u16, u16, u16);
    fn config_dir(&self) -> Result<PathBuf, String>;
    fn downloads_dir(&self) -> Option<PathBuf>;
    fn is_plain_directory(&self, meta: &std::fs::Metadata) -> bool;
    fn free_disk_bytes(&self, folder: &Path) -> Result<u64, String>;
    fn mark_of_the_web(&self, path: &Path) -> std::io::Result<()>;
    fn move_file_unique(
        &self,
        from: &Path,
        folder: &Path,
        name: &SafeName,
        transfer_id: Uuid,
    ) -> Result<String, String>;
    fn secret_file_name(&self) -> &'static str;
    fn protect_secret(&self, data: &[u8], decrypt: bool) -> Result<Vec<u8>, String>;
    fn save_secret_file(&self, path: &Path, bytes: &[u8]) -> Result<(), String>;
    fn open_url(&self, url: &str) -> Result<(), ()>;
    fn open_youtube_url(&self, url: &str) -> Result<(), ()>;
    fn open_folder(&self, folder: &Path) -> Result<(), ()>;
    fn pick_files(&self, owner: isize) -> Result<Vec<PathBuf>, String>;
    fn exit_after_update_install(&self) -> bool;
}

/// Returns the native owner window handle (HWND on Windows, 0 on Linux) for modal dialogs.
pub fn window_owner(app: &tauri::AppHandle) -> isize {
    #[cfg(windows)]
    {
        use tauri::Manager;
        app.get_webview_window("main")
            .and_then(|window| window.hwnd().ok())
            .map_or(0, |hwnd| hwnd.0 as isize)
    }
    #[cfg(target_os = "linux")]
    {
        let _ = app;
        0
    }
}

/// Returns the active platform's `InputBackend`.
pub fn input() -> &'static dyn InputBackend {
    #[cfg(windows)]
    {
        &windows::input::WindowsInput
    }
    #[cfg(target_os = "linux")]
    {
        &linux::input::LinuxInput
    }
}

/// Returns the active platform's `AudioBackend`.
pub fn audio() -> &'static dyn AudioBackend {
    #[cfg(windows)]
    {
        &windows::audio::WindowsAudio
    }
    #[cfg(target_os = "linux")]
    {
        &linux::audio::LinuxAudio
    }
}

/// Returns the active platform's `MediaBackend`.
pub fn media() -> &'static dyn MediaBackend {
    #[cfg(windows)]
    {
        &windows::media::WindowsMedia
    }
    #[cfg(target_os = "linux")]
    {
        &linux::media::LinuxMedia
    }
}

/// Returns the active platform's `PowerBackend`.
pub fn power() -> &'static dyn PowerBackend {
    #[cfg(windows)]
    {
        &windows::power::WindowsPower
    }
    #[cfg(target_os = "linux")]
    {
        &linux::power::LinuxPower
    }
}

/// Returns the active platform's `ClipboardBackend`.
pub fn clipboard() -> &'static dyn ClipboardBackend {
    #[cfg(windows)]
    {
        &windows::clipboard::WindowsClipboard
    }
    #[cfg(target_os = "linux")]
    {
        &linux::clipboard::LinuxClipboard
    }
}

/// Returns the active platform's `AutostartBackend`.
pub fn autostart() -> &'static dyn AutostartBackend {
    #[cfg(windows)]
    {
        &windows::autostart::WindowsAutostart
    }
    #[cfg(target_os = "linux")]
    {
        &linux::autostart::LinuxAutostart
    }
}

/// Returns the active platform's `SystemBackend`.
pub fn system() -> &'static dyn SystemBackend {
    #[cfg(windows)]
    {
        &windows::system::WindowsSystem
    }
    #[cfg(target_os = "linux")]
    {
        &linux::system::LinuxSystem
    }
}

/// Shared helper: readable player name for common AppUserModelIDs and MPRIS identities.
pub fn friendly_app(app_id: &str) -> String {
    let lower = app_id.to_ascii_lowercase();
    let known = [
        ("spotify", "Spotify"),
        ("chrome", "Google Chrome"),
        ("chromium", "Chromium"),
        ("msedge", "Microsoft Edge"),
        ("firefox", "Firefox"),
        ("308046b0af4a39cb", "Firefox"),
        ("opera", "Opera"),
        ("brave", "Brave"),
        ("vlc", "VLC"),
        ("mpv", "mpv"),
        ("rhythmbox", "Rhythmbox"),
        ("totem", "Vídeos"),
        ("celluloid", "Celluloid"),
        ("elisa", "Elisa"),
        ("amberol", "Amberol"),
        ("audacious", "Audacious"),
        ("clementine", "Clementine"),
        ("strawberry", "Strawberry"),
        ("microsoft.zunemusic", "Reprodutor Multimídia"),
        ("microsoft.zunevideo", "Filmes e TV"),
        ("wmplayer", "Windows Media Player"),
        ("msteams", "Microsoft Teams"),
        ("applemusic", "Apple Music"),
        ("itunes", "iTunes"),
        ("deezer", "Deezer"),
        ("tidal", "TIDAL"),
        ("amazon music", "Amazon Music"),
        ("foobar2000", "foobar2000"),
        ("mpc-hc", "MPC-HC"),
        ("potplayer", "PotPlayer"),
    ];
    known
        .iter()
        .find(|(needle, _)| lower.contains(needle))
        .map(|(_, name)| name.to_string())
        .unwrap_or_else(|| app_id.to_string())
}

/// Shared helper: validates that the clipboard sequence matches `expected` for automatic writes.
pub fn check_write_sequence(expected: Option<u32>, actual: u32) -> Result<(), PlatformError> {
    if expected.is_some_and(|sequence| sequence != actual) {
        return Err(PlatformError::Conflict);
    }
    Ok(())
}
