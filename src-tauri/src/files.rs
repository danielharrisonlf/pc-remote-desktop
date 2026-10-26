//! Receives files from the paired phone: PNG, JPEG and PDF up to 50 MB with
//! files.v1, any type up to 1 GiB with files.v2.
//!
//! The control session issues a short single-use ticket. Bytes arrive on the
//! separate `/v1/files` socket and are written to a part file that is renamed,
//! never overwriting, only after size, SHA-256 and (files.v1) content signature match.
use crate::{
    core::{constant_time_eq, now_ms, Core, Runtime, TransferDirection},
    protocol::{ClipboardOffer, MAX_FILE_BYTES, MAX_FILE_BYTES_V2, MAX_IMAGE_BYTES},
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, Notify};
use tokio_tungstenite::tungstenite::Bytes;
use uuid::Uuid;

pub const TICKET_TTL: Duration = Duration::from_secs(30);
pub const FOLDER_NAME: &str = "PC Remote";
pub const NOTIFICATION_TITLE: &str = "Arquivo recebido do celular";
const FREE_SPACE_MARGIN: u64 = 64 * 1024 * 1024;
const MAX_RECEIVED: usize = 12;
const MAX_STEM_CHARS: usize = 100;
const PART_SUFFIX: &str = ".pcremote-part";
const SIGNATURE_WINDOW: usize = 1024;

/// Extensions Explorer acts on just by listing the folder (icons, shell links),
/// refused even though files.v2 accepts any type. `desktop.ini` is checked by name.
const SHELL_ACTIVE: [&str; 8] = [
    "lnk",
    "url",
    "scf",
    "library-ms",
    "searchconnector-ms",
    "settingcontent-ms",
    "appref-ms",
    "website",
];
const MAX_MIME_BYTES: usize = 127;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Png,
    Jpeg,
    Pdf,
    /// files.v2: the content is not checked.
    Any,
}

impl FileKind {
    /// Checks the leading bytes; `head` holds at most the first 1024 bytes.
    pub fn signature_matches(self, head: &[u8]) -> bool {
        match self {
            Self::Png => head.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]),
            Self::Jpeg => head.starts_with(&[0xFF, 0xD8, 0xFF]),
            Self::Pdf => head[..head.len().min(SIGNATURE_WINDOW)]
                .windows(5)
                .any(|window| window == b"%PDF-"),
            Self::Any => true,
        }
    }
}

/// `type/subtype` in lowercase, at most 127 bytes, RFC 6838 restricted-name characters.
pub fn valid_mime(mime: &str) -> bool {
    let part = |value: &str| {
        !value.is_empty()
            && value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"!#$&^_.+-".contains(&b))
    };
    mime.len() <= MAX_MIME_BYTES
        && mime
            .split_once('/')
            .is_some_and(|(kind, subtype)| part(kind) && part(subtype))
}

/// MIME announced for a file the PC sends, from its extension.
pub fn mime_for(name: &SafeName) -> &'static str {
    match name.ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "csv" => "text/csv",
        "zip" => "application/zip",
        "mp4" => "video/mp4",
        "mkv" => "video/x-matroska",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "apk" => "application/vnd.android.package-archive",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => "application/octet-stream",
    }
}

fn shell_active(name: &SafeName) -> bool {
    SHELL_ACTIVE.contains(&name.ext.as_str())
        || name.file_name().eq_ignore_ascii_case("desktop.ini")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SafeName {
    pub stem: String,
    pub ext: String,
}

impl SafeName {
    pub fn file_name(&self) -> String {
        if self.ext.is_empty() {
            self.stem.clone()
        } else {
            format!("{}.{}", self.stem, self.ext)
        }
    }
    /// Shortens the stem (then the extension) until the whole name fits `max` UTF-8
    /// bytes, as `file.push` requires; Windows names may be longer than 255 bytes.
    pub fn fit(mut self, max: usize) -> Self {
        while self.file_name().len() > max && self.stem.chars().count() > 1 {
            self.stem.pop();
        }
        while self.file_name().len() > max && !self.ext.is_empty() {
            self.ext.pop();
        }
        self
    }
    /// `name.ext`, `name (1).ext` … `name (99).ext`, then `name <8 id chars>.ext`.
    pub fn candidates(&self, transfer_id: Uuid) -> impl Iterator<Item = String> + '_ {
        let suffix = transfer_id.to_string()[..8].to_string();
        std::iter::once(self.file_name())
            .chain((1..=99).map(move |n| self.with_stem_suffix(&format!(" ({n})"))))
            .chain(std::iter::once(
                self.with_stem_suffix(&format!(" {suffix}")),
            ))
    }
    fn with_stem_suffix(&self, suffix: &str) -> String {
        SafeName {
            stem: format!("{}{suffix}", self.stem),
            ext: self.ext.clone(),
        }
        .file_name()
    }
}

fn forbidden(c: char) -> bool {
    c.is_control()
        || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
        // Bidirectional overrides can disguise the real extension in Explorer.
        || matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

fn is_reserved(stem: &str) -> bool {
    let base = stem
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_uppercase();
    if matches!(
        base.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    let Some(rest) = base
        .strip_prefix("COM")
        .or_else(|| base.strip_prefix("LPT"))
    else {
        return false;
    };
    let mut chars = rest.chars();
    matches!(
        (chars.next(), chars.next()),
        (Some('0'..='9' | '¹' | '²' | '³'), None)
    )
}

/// Reduces a phone-provided name to a single safe Windows file name component.
pub fn sanitize_name(raw: &str) -> SafeName {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let mut cleaned = String::with_capacity(last.len());
    let mut pending_space = false;
    for c in last.chars().filter(|c| !forbidden(*c)) {
        if c.is_whitespace() {
            pending_space = true;
            continue;
        }
        if pending_space && !cleaned.is_empty() {
            cleaned.push(' ');
        }
        pending_space = false;
        cleaned.push(c);
    }
    let trimmed = cleaned.trim_matches(|c| c == ' ' || c == '.');
    let (stem, ext) = match trimmed.rfind('.') {
        Some(dot) => (&trimmed[..dot], trimmed[dot + 1..].to_lowercase()),
        None => (trimmed, String::new()),
    };
    let mut stem = if stem.is_empty() {
        "arquivo".to_string()
    } else {
        stem.to_string()
    };
    if is_reserved(&stem) {
        stem.insert(0, '_');
    }
    let mut stem: String = stem.chars().take(MAX_STEM_CHARS).collect();
    if ext.is_empty() {
        // Windows drops trailing spaces and dots, so the saved name would differ
        // from the one reported to the phone.
        stem.truncate(stem.trim_end_matches([' ', '.']).len());
        if stem.is_empty() {
            stem = "arquivo".to_string();
        }
    }
    SafeName { stem, ext }
}

/// Sanitizes the name and requires an allowed MIME type with a matching extension.
pub fn classify(name: &str, mime: &str) -> Result<(SafeName, FileKind), &'static str> {
    let safe = sanitize_name(name);
    let kind = match (mime, safe.ext.as_str()) {
        ("image/png", "png") => FileKind::Png,
        ("image/jpeg", "jpg" | "jpeg") => FileKind::Jpeg,
        ("application/pdf", "pdf") => FileKind::Pdf,
        _ => return Err("file_type"),
    };
    Ok((safe, kind))
}

fn offer_fields_valid(transfer_id: Uuid, name: &str, size: u64) -> bool {
    !transfer_id.is_nil()
        && !name.is_empty()
        && name.len() <= 255
        && !name.chars().any(char::is_control)
        && size > 0
}

/// Field rules of a files.v1 `file.offer`, in the order of the contract.
pub fn check_offer(
    transfer_id: Uuid,
    name: &str,
    size: u64,
    mime: &str,
) -> Result<(SafeName, FileKind), &'static str> {
    if !offer_fields_valid(transfer_id, name, size) {
        return Err("invalid_message");
    }
    if size > MAX_FILE_BYTES {
        return Err("too_large");
    }
    classify(name, mime)
}

/// files.v2 `file.offer`: any type up to 1 GiB; MIME is informative but must be well formed.
pub fn check_offer_v2(
    transfer_id: Uuid,
    name: &str,
    size: u64,
    mime: &str,
) -> Result<(SafeName, FileKind), &'static str> {
    if !offer_fields_valid(transfer_id, name, size) || !valid_mime(mime) {
        return Err("invalid_message");
    }
    if size > MAX_FILE_BYTES_V2 {
        return Err("too_large");
    }
    let safe = sanitize_name(name);
    if shell_active(&safe) {
        return Err("file_type");
    }
    Ok((safe, FileKind::Any))
}

/// clipboard.v2 `file.offer`: a PNG or JPEG of up to 32 MiB; the name is only informative.
pub fn check_clipboard_offer(
    transfer_id: Uuid,
    name: &str,
    size: u64,
    mime: &str,
) -> Result<FileKind, &'static str> {
    if !offer_fields_valid(transfer_id, name, size) {
        return Err("invalid_message");
    }
    let kind = match mime {
        "image/png" => FileKind::Png,
        "image/jpeg" => FileKind::Jpeg,
        _ => return Err("file_type"),
    };
    if size > MAX_IMAGE_BYTES {
        return Err("too_large");
    }
    Ok(kind)
}

/// Wall-clock budget of one transfer: 15 min, or 1 s per 256 KiB for large files.
pub fn total_timeout(size: u64, files_v2: bool) -> Duration {
    let base = Duration::from_secs(15 * 60);
    if files_v2 {
        base.max(Duration::from_secs(size.div_ceil(256 * 1024)))
    } else {
        base
    }
}

pub fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// clipboard.v2: an image from the phone that goes to the Windows clipboard, not Downloads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipTarget {
    pub event_id: Uuid,
    pub automatic: bool,
    /// Windows clipboard sequence at the offer; an automatic write needs it unchanged.
    pub expected_sequence: u32,
}

pub struct Ticket {
    secret: [u8; 32],
    device: Uuid,
    epoch: u64,
    transfer_id: Uuid,
    name: SafeName,
    size: u64,
    kind: FileKind,
    folder: PathBuf,
    expires: Instant,
    clipboard: Option<ClipTarget>,
}

pub struct Active {
    transfer_id: Uuid,
    device: Uuid,
    epoch: u64,
    name: String,
    size: u64,
    received: u64,
    abort: Arc<Notify>,
    /// Clipboard images are not files: the panel does not list them.
    clipboard: bool,
}

/// At most one ticket or transfer exists per PC.
pub enum Slot {
    Ticket(Ticket),
    Active(Active),
}

/// Everything the file socket needs after a ticket was redeemed.
pub struct Accepted {
    pub transfer_id: Uuid,
    pub device: Uuid,
    pub epoch: u64,
    pub name: SafeName,
    pub size: u64,
    pub kind: FileKind,
    pub folder: PathBuf,
    pub abort: Arc<Notify>,
    pub clipboard: Option<ClipTarget>,
}

/// Panel metadata only; the file content is never kept in memory or logs.
#[derive(Clone, Serialize)]
pub struct FileEntry {
    pub at: u64,
    pub name: String,
    pub bytes: u64,
    pub direction: TransferDirection,
}

#[derive(Clone, Serialize)]
pub struct IncomingFile {
    pub name: String,
    pub bytes: u64,
    pub received: u64,
}

impl Runtime {
    /// Drops the ticket or wakes the active transfer so it deletes its part file.
    pub(crate) fn abort_file(&mut self) {
        if let Some(Slot::Active(active)) = self.file_slot.take() {
            active.abort.notify_one();
        }
    }
    /// The session ended: nothing in flight survives, but the PC's outgoing queue waits.
    pub(crate) fn end_file_session(&mut self) {
        self.files_feature = false;
        self.files_v2 = false;
        self.abort_file();
        // A clipboard image never waits for the next connection; files do.
        self.drop_clipboard_push();
        self.abort_outgoing("A conexão com o celular terminou.");
    }
    pub(crate) fn record_file(&mut self, name: &str, bytes: u64, direction: TransferDirection) {
        if self.file_history.len() == MAX_RECEIVED {
            self.file_history.pop_front();
        }
        self.file_history.push_back(FileEntry {
            at: now_ms(),
            name: name.to_string(),
            bytes,
            direction,
        });
    }
    pub(crate) fn incoming_file(&self) -> Option<IncomingFile> {
        match &self.file_slot {
            Some(Slot::Active(active)) if !active.clipboard => Some(IncomingFile {
                name: active.name.clone(),
                bytes: active.size,
                received: active.received,
            }),
            _ => None,
        }
    }
    fn files_gate(&self, device: Uuid, epoch: u64) -> Result<(), &'static str> {
        if !self.session_ok(device, epoch) {
            return Err("unauthorized");
        }
        if !self.files_feature {
            return Err("unavailable");
        }
        Ok(())
    }
    fn file_busy(&mut self) -> bool {
        if matches!(&self.file_slot, Some(Slot::Ticket(t)) if t.expires <= Instant::now()) {
            self.file_slot = None;
        }
        self.file_slot.is_some()
    }
}

impl Core {
    /// Session and negotiation checks that precede field validation.
    pub fn file_gate(&self, device: Uuid, epoch: u64) -> Result<(), &'static str> {
        self.state.lock().unwrap().files_gate(device, epoch)
    }

    /// Whether this session negotiated files.v2.
    pub fn files_v2(&self, device: Uuid, epoch: u64) -> bool {
        let s = self.state.lock().unwrap();
        s.session_ok(device, epoch) && s.files_v2
    }

    /// Validates an offer and issues a ticket. Performs folder I/O; call from a blocking thread.
    /// With `clipboard` (clipboard.v2) the image waits in a temporary folder for the clipboard.
    #[allow(clippy::too_many_arguments)]
    pub fn file_offer(
        &self,
        device: Uuid,
        epoch: u64,
        transfer_id: Uuid,
        name: &str,
        size: u64,
        mime: &str,
        clipboard: Option<&ClipboardOffer>,
    ) -> Result<String, &'static str> {
        let (safe, kind, clipboard) = {
            let mut s = self.state.lock().unwrap();
            s.files_gate(device, epoch)?;
            let (safe, kind, target) = match clipboard {
                Some(offer) => {
                    if !s.clipboard_image {
                        return Err("invalid_message");
                    }
                    let kind = check_clipboard_offer(transfer_id, name, size, mime)?;
                    s.clipboard_image_consent(
                        offer.event_id,
                        offer.automatic,
                        Some(offer.base_revision),
                    )?;
                    let target = ClipTarget {
                        event_id: offer.event_id,
                        automatic: offer.automatic,
                        expected_sequence: s.clipboard_sequence,
                    };
                    (sanitize_name(name), kind, Some(target))
                }
                None if s.files_v2 => {
                    let (safe, kind) = check_offer_v2(transfer_id, name, size, mime)?;
                    (safe, kind, None)
                }
                None => {
                    let (safe, kind) = check_offer(transfer_id, name, size, mime)?;
                    (safe, kind, None)
                }
            };
            if s.file_busy() {
                return Err("file_busy");
            }
            (safe, kind, target)
        };
        // Disk work happens without the runtime lock; the slot is re-checked below.
        let folder = if clipboard.is_some() {
            self.clipboard_folder().map_err(|error| {
                self.diagnostic(&format!(
                    "Pasta temporária indisponível para imagens do celular: {error}"
                ));
                "storage"
            })?
        } else {
            self.prepare_folder().map_err(|error| {
                self.diagnostic(&format!(
                    "Pasta Downloads\\{FOLDER_NAME} indisponível para receber arquivos: {error}"
                ));
                "storage"
            })?
        };
        if free_bytes(&folder).map_or(true, |free| free < size.saturating_add(FREE_SPACE_MARGIN)) {
            self.diagnostic("Arquivo do celular recusado: pouco espaço livre no disco.");
            return Err("storage");
        }
        let mut secret = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::rng(), &mut secret);
        let mut s = self.state.lock().unwrap();
        s.files_gate(device, epoch)?;
        if s.file_busy() {
            return Err("file_busy");
        }
        s.file_slot = Some(Slot::Ticket(Ticket {
            secret,
            device,
            epoch,
            transfer_id,
            name: safe,
            size,
            kind,
            folder,
            expires: Instant::now() + TICKET_TTL,
            clipboard,
        }));
        Ok(hex::encode(secret))
    }

    /// Drops an unused ticket or aborts the transfer with this id. Unknown ids are ignored.
    pub fn file_cancel(&self, device: Uuid, epoch: u64, transfer_id: Uuid) {
        let mut s = self.state.lock().unwrap();
        let owned = match &s.file_slot {
            Some(Slot::Ticket(t)) => {
                t.transfer_id == transfer_id && t.device == device && t.epoch == epoch
            }
            Some(Slot::Active(a)) => {
                a.transfer_id == transfer_id && a.device == device && a.epoch == epoch
            }
            None => false,
        };
        if owned {
            s.abort_file();
        }
    }

    /// Redeems a ticket. It is consumed by any attempt that names its transfer.
    pub fn file_begin(&self, transfer_id: Uuid, presented: &str) -> Result<Accepted, &'static str> {
        let mut s = self.state.lock().unwrap();
        if !matches!(&s.file_slot, Some(Slot::Ticket(t)) if t.transfer_id == transfer_id) {
            return Err("unauthorized");
        }
        let Some(Slot::Ticket(ticket)) = s.file_slot.take() else {
            return Err("unauthorized");
        };
        let presented = is_sha256_hex(presented)
            .then(|| hex::decode(presented).ok())
            .flatten()
            .unwrap_or_default();
        let matches = constant_time_eq(&presented, &ticket.secret);
        if !matches || ticket.expires <= Instant::now() {
            return Err("unauthorized");
        }
        if !s.session_ok(ticket.device, ticket.epoch) {
            return Err("unauthorized");
        }
        if !s.files_feature {
            return Err("unavailable");
        }
        let abort = Arc::new(Notify::new());
        s.file_slot = Some(Slot::Active(Active {
            transfer_id,
            device: ticket.device,
            epoch: ticket.epoch,
            name: ticket.name.file_name(),
            size: ticket.size,
            received: 0,
            abort: abort.clone(),
            clipboard: ticket.clipboard.is_some(),
        }));
        Ok(Accepted {
            transfer_id,
            device: ticket.device,
            epoch: ticket.epoch,
            name: ticket.name,
            size: ticket.size,
            kind: ticket.kind,
            folder: ticket.folder,
            abort,
            clipboard: ticket.clipboard,
        })
    }

    /// Per-frame authorization check; also records progress for the panel.
    /// `cancelled` means the phone dropped this transfer through `file.cancel`.
    pub fn file_check(&self, accepted: &Accepted, received: u64) -> Result<(), &'static str> {
        let mut s = self.state.lock().unwrap();
        s.files_gate(accepted.device, accepted.epoch)?;
        match &mut s.file_slot {
            Some(Slot::Active(active)) if Arc::ptr_eq(&active.abort, &accepted.abort) => {
                active.received = received;
                Ok(())
            }
            _ => Err("cancelled"),
        }
    }

    /// Clears the slot if it still belongs to this activation (`Accepted::abort`).
    pub fn file_release(&self, token: &Arc<Notify>) {
        let mut s = self.state.lock().unwrap();
        if matches!(&s.file_slot, Some(Slot::Active(a)) if Arc::ptr_eq(&a.abort, token)) {
            s.file_slot = None;
        }
    }

    pub fn file_completed(&self, token: &Arc<Notify>, name: &str, bytes: u64) {
        {
            let mut s = self.state.lock().unwrap();
            if matches!(&s.file_slot, Some(Slot::Active(a)) if Arc::ptr_eq(&a.abort, token)) {
                s.file_slot = None;
            }
            s.record_file(name, bytes, TransferDirection::ToPc);
        }
        if let Some(notify) = self.notifier.get() {
            notify(
                NOTIFICATION_TITLE,
                &format!("{name} foi salvo em Downloads\\{FOLDER_NAME}."),
            );
        }
    }

    /// `Downloads\PC Remote`, created on demand and required to be a plain directory.
    pub fn received_folder(&self) -> Result<PathBuf, String> {
        let base = match &self.files_root {
            Some(root) => root.clone(),
            None => downloads_folder().ok_or("Pasta Downloads não encontrada")?,
        };
        let folder = base.join(FOLDER_NAME);
        ensure_plain_folder(&folder)?;
        Ok(folder)
    }

    /// Temporary clipboard images in both directions; never Downloads.
    pub(crate) fn clipboard_folder(&self) -> Result<PathBuf, String> {
        let folder = match &self.files_root {
            Some(root) => root.join("clipboard-temp"),
            None => std::env::temp_dir().join(FOLDER_NAME),
        };
        ensure_plain_folder(&folder)?;
        Ok(folder)
    }

    fn prepare_folder(&self) -> Result<PathBuf, String> {
        let folder = self.received_folder()?;
        // Before the first transfer of this process nothing can be in flight.
        if !self.files_cleaned.swap(true, Ordering::AcqRel) {
            remove_stale_parts(&folder);
        }
        Ok(folder)
    }
}

fn downloads_folder() -> Option<PathBuf> {
    crate::platform::system().downloads_dir()
}

fn plain_directory(meta: &fs::Metadata) -> bool {
    crate::platform::system().is_plain_directory(meta)
}

pub fn ensure_plain_folder(folder: &Path) -> Result<(), String> {
    let meta = match fs::symlink_metadata(folder) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(folder).map_err(|e| e.to_string())?;
            fs::symlink_metadata(folder).map_err(|e| e.to_string())?
        }
        Err(error) => return Err(error.to_string()),
    };
    if plain_directory(&meta) {
        Ok(())
    } else {
        Err("o destino não é uma pasta comum".into())
    }
}

fn remove_stale_parts(folder: &Path) {
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let stale = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.ends_with(PART_SUFFIX))
            && entry.file_type().is_ok_and(|kind| kind.is_file());
        if stale {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn free_bytes(folder: &Path) -> Result<u64, String> {
    crate::platform::system().free_disk_bytes(folder)
}

pub fn part_path(folder: &Path, transfer_id: Uuid) -> PathBuf {
    folder.join(format!(".{transfer_id}{PART_SUFFIX}"))
}

pub fn create_part(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

pub struct Written {
    file: File,
    pub digest: [u8; 32],
}

/// Blocking writer: hashes and buffers chunks until the sender is dropped.
pub fn write_part(file: File, mut chunks: mpsc::Receiver<Bytes>) -> io::Result<Written> {
    let mut hasher = Sha256::new();
    let mut out = BufWriter::with_capacity(256 * 1024, file);
    while let Some(chunk) = chunks.blocking_recv() {
        hasher.update(&chunk);
        out.write_all(&chunk)?;
    }
    let file = out.into_inner().map_err(|e| e.into_error())?;
    Ok(Written {
        file,
        digest: hasher.finalize().into(),
    })
}

/// Blocking reader for PC → phone: sends exactly `size` bytes in 64 KiB chunks and
/// returns their SHA-256. A file that shrank or grew since it was queued is an error.
pub fn read_for_send(
    path: &Path,
    size: u64,
    chunks: mpsc::Sender<Bytes>,
) -> Result<[u8; 32], String> {
    use std::io::Read;
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut left = size;
    while left > 0 {
        let mut buffer = vec![0u8; left.min(crate::protocol::FILE_CHUNK_BYTES as u64) as usize];
        let mut filled = 0;
        while filled < buffer.len() {
            match file.read(&mut buffer[filled..]) {
                Ok(0) => return Err("o arquivo ficou menor".into()),
                Ok(n) => filled += n,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        hasher.update(&buffer);
        left -= buffer.len() as u64;
        // The socket side stopped: cancelled, failed or closed.
        chunks
            .blocking_send(Bytes::from(buffer))
            .map_err(|_| "envio interrompido".to_string())?;
    }
    let mut probe = [0u8; 1];
    if file.read(&mut probe).map_err(|e| e.to_string())? != 0 {
        return Err("o arquivo ficou maior".into());
    }
    Ok(hasher.finalize().into())
}

fn mark_of_the_web(path: &Path) -> io::Result<()> {
    crate::platform::system().mark_of_the_web(path)
}

/// Flushes, marks as downloaded from the internet and moves to a free name.
pub fn finalize(
    written: Written,
    part: &Path,
    folder: &Path,
    name: &SafeName,
    transfer_id: Uuid,
) -> Result<String, String> {
    written.file.sync_all().map_err(|e| e.to_string())?;
    drop(written.file);
    mark_of_the_web(part).map_err(|e| format!("marca de origem: {e}"))?;
    ensure_plain_folder(folder)?;
    move_unique(part, folder, name, transfer_id)
}

/// Never replaces an existing file.
pub fn move_unique(
    from: &Path,
    folder: &Path,
    name: &SafeName,
    transfer_id: Uuid,
) -> Result<String, String> {
    crate::platform::system().move_file_unique(from, folder, name, transfer_id)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        protocol::Features,
        secrets::{self, Device, Secrets},
    };

    pub(crate) fn temp_folder(label: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("pc-remote-{label}-{}", Uuid::new_v4()));
        fs::create_dir_all(&folder).unwrap();
        folder
    }

    fn name(raw: &str) -> String {
        sanitize_name(raw).file_name()
    }

    #[test]
    fn names_keep_only_the_last_component_and_drop_forbidden_characters() {
        assert_eq!(name("../../Windows/System32/evil.jpg"), "evil.jpg");
        assert_eq!(name("..\\..\\boot.ini\\foto.PNG"), "foto.png");
        assert_eq!(name("C:\\Users\\a\\doc.pdf"), "doc.pdf");
        assert_eq!(name("a<b>c:d\"e|f?g*h.jpg"), "abcdefgh.jpg");
        assert_eq!(name("con\u{0}trol\u{7}.png"), "control.png");
        assert_eq!(name("recibo\u{202E}fdp.png"), "recibofdp.png");
        assert_eq!(name("/"), "arquivo");
        assert_eq!(name(""), "arquivo");
    }

    #[test]
    fn names_collapse_whitespace_and_trim_dots_and_spaces() {
        assert_eq!(
            name("  minha \t  foto\u{00A0}\u{3000}nova .JPEG.. "),
            "minha foto nova .jpeg"
        );
        assert_eq!(name("...arquivo.pdf..."), "arquivo.pdf");
        assert_eq!(name(". . .png"), "png");
        assert_eq!(sanitize_name("sem extensão").ext, "");
        assert_eq!(name("a.b.c.PDF"), "a.b.c.pdf");
    }

    #[test]
    fn truncation_never_leaves_a_name_windows_would_change() {
        let safe = sanitize_name(&format!("{} b", "a".repeat(MAX_STEM_CHARS - 1)));
        assert_eq!(safe.file_name(), "a".repeat(MAX_STEM_CHARS - 1));
        // Before an extension the space is harmless and kept.
        let safe = sanitize_name(&format!("{} b.pdf", "a".repeat(MAX_STEM_CHARS - 1)));
        assert_eq!(
            safe.file_name(),
            format!("{} .pdf", "a".repeat(MAX_STEM_CHARS - 1))
        );
    }

    #[test]
    fn reserved_device_names_are_prefixed_even_with_extensions() {
        for (raw, expected) in [
            ("CON.jpg", "_CON.jpg"),
            ("nul.png", "_nul.png"),
            ("Aux.tar.pdf", "_Aux.tar.pdf"),
            ("prn .jpg", "_prn .jpg"),
            ("COM0.jpg", "_COM0.jpg"),
            ("lpt9.pdf", "_lpt9.pdf"),
            ("COM¹.png", "_COM¹.png"),
            ("conout$.png", "_conout$.png"),
        ] {
            assert_eq!(name(raw), expected, "{raw}");
        }
        for raw in ["CONSOLE.jpg", "COM10.jpg", "LPT.pdf", "nullo.png"] {
            assert_eq!(name(raw), raw, "{raw}");
        }
    }

    #[test]
    fn stems_are_truncated_on_character_boundaries() {
        let long = format!("{}.png", "é🦊".repeat(80));
        let safe = sanitize_name(&long);
        assert_eq!(safe.stem.chars().count(), MAX_STEM_CHARS);
        assert_eq!(safe.ext, "png");
        assert!(safe.stem.ends_with('🦊'));
        let reserved = sanitize_name(&format!("CON.{}.jpg", "x".repeat(200)));
        assert!(reserved.stem.starts_with("_CON."));
        assert_eq!(reserved.stem.chars().count(), MAX_STEM_CHARS);
    }

    #[test]
    fn mime_and_extension_must_match() {
        let allowed = [
            ("a.png", "image/png", FileKind::Png),
            ("a.PNG", "image/png", FileKind::Png),
            ("a.jpg", "image/jpeg", FileKind::Jpeg),
            ("a.jpeg", "image/jpeg", FileKind::Jpeg),
            ("a.JPG", "image/jpeg", FileKind::Jpeg),
            ("a.pdf", "application/pdf", FileKind::Pdf),
        ];
        for (file, mime, kind) in allowed {
            assert_eq!(
                classify(file, mime).map(|(_, k)| k),
                Ok(kind),
                "{file} {mime}"
            );
        }
        for (file, mime) in [
            ("a.png", "image/jpeg"),
            ("a.jpg", "image/png"),
            ("a.pdf", "image/png"),
            ("a.png", "application/pdf"),
            ("a.gif", "image/gif"),
            ("a.exe", "application/pdf"),
            ("a.pdf.exe", "application/pdf"),
            ("a.png", "IMAGE/PNG"),
            ("a.png", "image/png; charset=binary"),
            ("a", "image/png"),
            ("a.svg", "image/svg+xml"),
            ("a.webp", "image/webp"),
        ] {
            assert_eq!(
                classify(file, mime).map(|_| ()),
                Err("file_type"),
                "{file} {mime}"
            );
        }
    }

    #[test]
    fn content_signatures() {
        let png = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0];
        assert!(FileKind::Png.signature_matches(&png));
        assert!(!FileKind::Png.signature_matches(&png[..7]));
        assert!(!FileKind::Jpeg.signature_matches(&png));
        assert!(FileKind::Jpeg.signature_matches(&[0xFF, 0xD8, 0xFF, 0xE0]));
        assert!(!FileKind::Jpeg.signature_matches(&[0xFF, 0xD8]));
        assert!(FileKind::Pdf.signature_matches(b"%PDF-1.7\n"));
        let mut late = vec![b' '; 1019];
        late.extend_from_slice(b"%PDF-");
        assert!(FileKind::Pdf.signature_matches(&late));
        let mut too_late = vec![b' '; 1020];
        too_late.extend_from_slice(b"%PDF-");
        assert!(!FileKind::Pdf.signature_matches(&too_late));
        assert!(!FileKind::Pdf.signature_matches(b"%PDF"));
        assert!(!FileKind::Pdf.signature_matches(&png));
    }

    #[test]
    fn unique_names_never_overwrite_existing_files() {
        let folder = temp_folder("unique");
        let safe = sanitize_name("foto.jpg");
        let id = Uuid::new_v4();
        fs::write(folder.join("foto.jpg"), b"original").unwrap();
        fs::write(folder.join("FOTO (1).JPG"), b"second").unwrap();
        let part = part_path(&folder, id);
        fs::write(&part, b"new").unwrap();
        assert_eq!(
            move_unique(&part, &folder, &safe, id).unwrap(),
            "foto (2).jpg"
        );
        assert_eq!(fs::read(folder.join("foto.jpg")).unwrap(), b"original");
        assert_eq!(fs::read(folder.join("FOTO (1).JPG")).unwrap(), b"second");
        assert_eq!(fs::read(folder.join("foto (2).jpg")).unwrap(), b"new");
        assert!(!part.exists());
        for n in 3..=99 {
            fs::write(folder.join(format!("foto ({n}).jpg")), b"x").unwrap();
        }
        fs::write(&part, b"last").unwrap();
        let fallback = format!("foto {}.jpg", &id.to_string()[..8]);
        assert_eq!(move_unique(&part, &folder, &safe, id).unwrap(), fallback);
        fs::write(&part, b"none").unwrap();
        assert!(move_unique(&part, &folder, &safe, id).is_err());
        assert_eq!(fs::read(&part).unwrap(), b"none");
        assert_eq!(fs::read(folder.join(&fallback)).unwrap(), b"last");
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn stale_part_files_are_removed_and_other_files_kept() {
        let folder = temp_folder("stale");
        let stale = part_path(&folder, Uuid::new_v4());
        fs::write(&stale, b"partial").unwrap();
        fs::write(folder.join("foto.png"), b"keep").unwrap();
        fs::create_dir(folder.join("dir.pcremote-part")).unwrap();
        remove_stale_parts(&folder);
        assert!(!stale.exists());
        assert!(folder.join("foto.png").exists());
        assert!(folder.join("dir.pcremote-part").is_dir());
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn target_folder_must_be_a_plain_directory() {
        let root = temp_folder("plain");
        let folder = root.join(FOLDER_NAME);
        ensure_plain_folder(&folder).unwrap();
        assert!(folder.is_dir());
        ensure_plain_folder(&folder).unwrap();
        let file = root.join("arquivo");
        fs::write(&file, b"x").unwrap();
        assert!(ensure_plain_folder(&file).is_err());
        let junction = root.join("junction");
        let created = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&folder)
            .output()
            .is_ok_and(|output| output.status.success());
        if created {
            assert!(ensure_plain_folder(&junction).is_err());
            fs::remove_dir(&junction).unwrap();
        } else {
            eprintln!("SKIP: could not create a directory junction");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn downloads_known_folder_resolves_without_com_initialization() {
        // Read-only: resolves the path on a fresh thread, like the blocking pool, and creates nothing.
        let folder = std::thread::spawn(downloads_folder).join().unwrap();
        let folder = folder.expect("Downloads folder");
        assert!(folder.is_absolute(), "{}", folder.display());
    }

    #[test]
    fn writer_hashes_and_marks_the_final_file() {
        let folder = temp_folder("writer");
        let id = Uuid::new_v4();
        let part = part_path(&folder, id);
        let file = create_part(&part).unwrap();
        assert!(create_part(&part).is_err());
        let (tx, rx) = mpsc::channel(4);
        let writer = std::thread::spawn(move || write_part(file, rx));
        tx.blocking_send(Bytes::from_static(b"%PDF-1.4 ")).unwrap();
        tx.blocking_send(Bytes::from_static(b"conteudo")).unwrap();
        drop(tx);
        let written = writer.join().unwrap().unwrap();
        assert_eq!(
            written.digest,
            <[u8; 32]>::from(Sha256::digest(b"%PDF-1.4 conteudo"))
        );
        let saved = finalize(written, &part, &folder, &sanitize_name("doc.pdf"), id).unwrap();
        assert_eq!(saved, "doc.pdf");
        assert_eq!(
            fs::read(folder.join("doc.pdf")).unwrap(),
            b"%PDF-1.4 conteudo"
        );
        if let Ok(motw) = fs::read(folder.join("doc.pdf:Zone.Identifier")) {
            assert_eq!(motw, b"[ZoneTransfer]\r\nZoneId=3\r\n");
        }
        fs::remove_dir_all(folder).unwrap();
    }

    fn session_core(files: bool) -> (Arc<Core>, Uuid, String, u64) {
        let core = Core::ephemeral(
            Secrets {
                pc_id: Uuid::new_v4(),
                certificate_pem: String::new(),
                private_key_pem: String::new(),
                devices: Vec::new(),
            },
            "00".into(),
        );
        let id = Uuid::new_v4();
        let token = crate::core::random_hex();
        core.state.lock().unwrap().secrets.devices.push(Device {
            id,
            name: "Files test".into(),
            token_hash: secrets::token_hash(&token),
        });
        let epoch = core
            .connect_authenticated(id, &token, Features::legacy(false, files))
            .unwrap();
        (core, id, token, epoch)
    }

    fn offer(
        core: &Core,
        device: Uuid,
        epoch: u64,
        transfer: Uuid,
    ) -> Result<String, &'static str> {
        core.file_offer(device, epoch, transfer, "foto.png", 10, "image/png", None)
    }

    fn cleanup(core: &Core) {
        if let Some(root) = &core.files_root {
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn tickets_require_negotiation_permission_and_a_free_slot() {
        let (core, id, token, epoch) = session_core(false);
        assert_eq!(offer(&core, id, epoch, Uuid::new_v4()), Err("unavailable"));
        let epoch = core
            .connect_authenticated(id, &token, Features::legacy(false, true))
            .unwrap();
        assert_eq!(
            offer(&core, id, epoch + 1, Uuid::new_v4()),
            Err("unauthorized")
        );
        assert_eq!(
            core.file_offer(
                id,
                epoch,
                Uuid::new_v4(),
                "a.png",
                MAX_FILE_BYTES + 1,
                "image/png",
                None
            ),
            Err("too_large")
        );
        assert_eq!(
            core.file_offer(id, epoch, Uuid::new_v4(), "a.png", 10, "image/jpeg", None),
            Err("file_type")
        );
        let ticket = offer(&core, id, epoch, Uuid::new_v4()).unwrap();
        assert!(is_sha256_hex(&ticket));
        assert_eq!(offer(&core, id, epoch, Uuid::new_v4()), Err("file_busy"));
        cleanup(&core);
    }

    #[test]
    fn tickets_are_single_use_short_lived_and_epoch_bound() {
        let (core, id, token, epoch) = session_core(true);
        let transfer = Uuid::new_v4();
        let ticket = offer(&core, id, epoch, transfer).unwrap();
        // Another transfer id does not consume this ticket.
        assert_eq!(
            core.file_begin(Uuid::new_v4(), &ticket).err(),
            Some("unauthorized")
        );
        let accepted = core.file_begin(transfer, &ticket).unwrap();
        assert_eq!(accepted.name.file_name(), "foto.png");
        assert_eq!(core.status().incoming_file.unwrap().bytes, 10);
        assert_eq!(
            core.file_begin(transfer, &ticket).err(),
            Some("unauthorized")
        );
        core.file_check(&accepted, 4).unwrap();
        assert_eq!(core.status().incoming_file.unwrap().received, 4);
        core.file_release(&accepted.abort);
        assert!(core.status().incoming_file.is_none());

        // A wrong secret burns the ticket; the correct one then fails too.
        let transfer = Uuid::new_v4();
        let ticket = offer(&core, id, epoch, transfer).unwrap();
        let mut wrong = ticket.clone().into_bytes();
        wrong[63] = if wrong[63] == b'0' { b'1' } else { b'0' };
        let wrong = String::from_utf8(wrong).unwrap();
        assert_eq!(
            core.file_begin(transfer, &wrong).err(),
            Some("unauthorized")
        );
        assert_eq!(
            core.file_begin(transfer, &ticket).err(),
            Some("unauthorized")
        );
        assert_eq!(core.file_begin(transfer, "XYZ").err(), Some("unauthorized"));

        // Expired tickets are refused and no longer block new offers.
        let transfer = Uuid::new_v4();
        let ticket = offer(&core, id, epoch, transfer).unwrap();
        if let Some(Slot::Ticket(t)) = &mut core.state.lock().unwrap().file_slot {
            t.expires = Instant::now() - Duration::from_millis(1);
        }
        let next = Uuid::new_v4();
        let next_ticket = offer(&core, id, epoch, next).unwrap();
        assert_eq!(
            core.file_begin(transfer, &ticket).err(),
            Some("unauthorized")
        );

        // A reconnection changes the epoch and invalidates the ticket.
        let epoch = core
            .connect_authenticated(id, &token, Features::legacy(false, true))
            .unwrap();
        assert_eq!(
            core.file_begin(next, &next_ticket).err(),
            Some("unauthorized")
        );
        let transfer = Uuid::new_v4();
        let ticket = offer(&core, id, epoch, transfer).unwrap();
        core.disconnect(id, epoch);
        assert_eq!(
            core.file_begin(transfer, &ticket).err(),
            Some("unauthorized")
        );
        cleanup(&core);
    }

    #[test]
    fn active_transfers_abort_on_cancel_permission_and_session_changes() {
        let (core, id, token, epoch) = session_core(true);
        let transfer = Uuid::new_v4();
        let ticket = offer(&core, id, epoch, transfer).unwrap();
        let accepted = core.file_begin(transfer, &ticket).unwrap();
        core.file_cancel(id, epoch, Uuid::new_v4());
        core.file_check(&accepted, 1).unwrap();
        core.file_cancel(id, epoch, transfer);
        assert_eq!(core.file_check(&accepted, 2), Err("cancelled"));
        // A late cleanup of the old activation cannot free a new one reusing the id.
        let ticket = offer(&core, id, epoch, transfer).unwrap();
        let reused = core.file_begin(transfer, &ticket).unwrap();
        core.file_release(&accepted.abort);
        assert_eq!(core.file_check(&accepted, 3), Err("cancelled"));
        core.file_check(&reused, 1).unwrap();
        core.file_release(&reused.abort);
        assert!(core.status().incoming_file.is_none());

        // Changing the sync preference never touches a file in flight.
        let transfer = Uuid::new_v4();
        let ticket = offer(&core, id, epoch, transfer).unwrap();
        let accepted = core.file_begin(transfer, &ticket).unwrap();
        let mut settings = core.status().settings;
        settings.realtime = false;
        core.update_settings_with_io(settings, |_| Ok(()), |_| Ok(()))
            .unwrap();
        core.file_check(&accepted, 1).unwrap();
        assert!(core.snapshot().files.is_some_and(|files| files.allowed));
        core.file_release(&accepted.abort);

        let transfer = Uuid::new_v4();
        let ticket = offer(&core, id, epoch, transfer).unwrap();
        let accepted = core.file_begin(transfer, &ticket).unwrap();
        core.connect_authenticated(id, &token, Features::legacy(false, true))
            .unwrap();
        assert_eq!(core.file_check(&accepted, 1), Err("unauthorized"));
        assert!(core.status().incoming_file.is_none());
        cleanup(&core);
    }

    #[test]
    fn received_history_is_metadata_only_bounded_and_notifies() {
        let (core, _, _, _) = session_core(true);
        let shown = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = shown.clone();
        core.set_notifier(Box::new(move |title, body| {
            sink.lock()
                .unwrap()
                .push((title.to_string(), body.to_string()));
        }));
        for index in 0..(MAX_RECEIVED + 2) {
            core.file_completed(&Arc::new(Notify::new()), &format!("foto {index}.png"), 42);
        }
        let status = core.status();
        assert_eq!(status.file_history.len(), MAX_RECEIVED);
        assert_eq!(
            status.file_history[0].name,
            format!("foto {}.png", MAX_RECEIVED + 1)
        );
        assert_eq!(status.file_history[0].direction, TransferDirection::ToPc);
        let shown = shown.lock().unwrap();
        assert_eq!(shown.len(), MAX_RECEIVED + 2);
        assert_eq!(shown[0].0, "Arquivo recebido do celular");
        assert_eq!(shown[0].1, "foto 0.png foi salvo em Downloads\\PC Remote.");
        cleanup(&core);
    }

    #[test]
    fn v2_offers_take_any_type_up_to_one_gigabyte_except_shell_active_files() {
        let id = Uuid::new_v4();
        for (file, mime) in [
            ("filme.mkv", "video/x-matroska"),
            ("setup.exe", "application/octet-stream"),
            ("sem extensão", "application/octet-stream"),
            ("foto.png", "image/jpeg"),
            (
                "planilha.xlsx",
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            ),
        ] {
            let (safe, kind) = check_offer_v2(id, file, MAX_FILE_BYTES_V2, mime).unwrap();
            assert_eq!(kind, FileKind::Any);
            assert_eq!(safe.file_name(), sanitize_name(file).file_name());
        }
        assert_eq!(
            check_offer_v2(id, "a.mkv", MAX_FILE_BYTES_V2 + 1, "video/x-matroska").err(),
            Some("too_large")
        );
        for name in [
            "atalho.lnk",
            "SITE.URL",
            "x.scf",
            "pasta.library-ms",
            "busca.searchConnector-ms",
            "a.settingcontent-ms",
            "app.appref-ms",
            "b.website",
            "desktop.ini",
            "Desktop.INI",
            "..\\desktop.ini",
        ] {
            assert_eq!(
                check_offer_v2(id, name, 10, "application/octet-stream").err(),
                Some("file_type"),
                "{name}"
            );
        }
        for mime in [
            "",
            "video",
            "video/",
            "/mp4",
            "Video/MP4",
            "video/mp4; x=1",
            "a/b/c",
            "vídeo/mp4",
        ] {
            assert_eq!(
                check_offer_v2(id, "a.mp4", 10, mime).err(),
                Some("invalid_message"),
                "{mime}"
            );
        }
        assert!(!valid_mime(&format!("a/{}", "b".repeat(126))));
        assert!(valid_mime(&format!("a/{}", "b".repeat(125))));
        for (field, size) in [("", 1), ("a\n.mp4", 1), ("a.mp4", 0)] {
            assert_eq!(
                check_offer_v2(id, field, size, "video/mp4").err(),
                Some("invalid_message")
            );
        }
        assert_eq!(
            check_offer_v2(Uuid::nil(), "a.mp4", 1, "video/mp4").err(),
            Some("invalid_message")
        );
        // files.v1 stays exactly as before.
        assert_eq!(
            check_offer(id, "filme.mkv", 10, "video/x-matroska").err(),
            Some("file_type")
        );
        assert_eq!(
            check_offer(id, "a.png", MAX_FILE_BYTES + 1, "image/png").err(),
            Some("too_large")
        );
    }

    #[test]
    fn pushed_names_fit_255_bytes_on_character_boundaries() {
        let long = sanitize_name(&format!("{}.pdf", "漢".repeat(100)));
        assert_eq!(long.file_name().len(), 304);
        let fitted = long.fit(255);
        assert!(fitted.file_name().len() <= 255);
        assert_eq!(fitted.ext, "pdf");
        assert!(fitted.stem.chars().all(|c| c == '漢'));
        let ext = SafeName {
            stem: "a".into(),
            ext: "é".repeat(200),
        }
        .fit(255);
        assert!(ext.file_name().len() <= 255 && ext.stem == "a");
        assert_eq!(sanitize_name("curto.txt").fit(255).file_name(), "curto.txt");
    }

    #[test]
    fn announced_mime_types_are_well_formed_and_large_files_get_more_time() {
        for ext in [
            "png", "jpg", "pdf", "mp4", "mkv", "docx", "xlsx", "pptx", "apk", "zip", "x", "",
        ] {
            let name = SafeName {
                stem: "a".into(),
                ext: ext.into(),
            };
            assert!(valid_mime(mime_for(&name)), "{ext}");
        }
        assert_eq!(
            total_timeout(MAX_FILE_BYTES, false),
            Duration::from_secs(900)
        );
        assert_eq!(
            total_timeout(MAX_FILE_BYTES_V2, false),
            Duration::from_secs(900)
        );
        assert_eq!(total_timeout(10, true), Duration::from_secs(900));
        assert_eq!(
            total_timeout(MAX_FILE_BYTES_V2, true),
            Duration::from_secs(4096)
        );
    }

    #[test]
    fn v2_sessions_receive_any_type_and_v1_sessions_keep_the_old_rules() {
        let (core, id, token, epoch) = session_core(true);
        assert_eq!(
            core.file_offer(
                id,
                epoch,
                Uuid::new_v4(),
                "filme.mkv",
                10,
                "video/x-matroska",
                None
            ),
            Err("file_type")
        );
        let features = Features {
            files: true,
            files_v2: true,
            ..Features::default()
        };
        let epoch = core.connect_authenticated(id, &token, features).unwrap();
        let transfer = Uuid::new_v4();
        let ticket = core
            .file_offer(
                id,
                epoch,
                transfer,
                "filme.mkv",
                MAX_FILE_BYTES + 1,
                "video/x-matroska",
                None,
            )
            .unwrap();
        let accepted = core.file_begin(transfer, &ticket).unwrap();
        assert_eq!(accepted.kind, FileKind::Any);
        assert_eq!(accepted.name.file_name(), "filme.mkv");
        core.file_release(&accepted.abort);
        assert_eq!(
            core.file_offer(
                id,
                epoch,
                Uuid::new_v4(),
                "a.lnk",
                10,
                "application/octet-stream",
                None
            ),
            Err("file_type")
        );
        cleanup(&core);
    }
}
