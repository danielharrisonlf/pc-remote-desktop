use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const PORT: u16 = 43127;
pub const MAX_FRAME: usize = 512 * 1024;
pub const MAX_TEXT: usize = 64 * 1024;
/// files.v1 limit; clients that did not negotiate files.v2 keep it.
pub const MAX_FILE_BYTES: u64 = 52_428_800;
/// files.v2 limit, in both directions.
pub const MAX_FILE_BYTES_V2: u64 = 1_073_741_824;
pub const FILE_CHUNK_BYTES: usize = 65_536;
pub const FILE_WINDOW_BYTES: usize = 1_048_576;
pub const FILE_TEXT_LIMIT: usize = 4_096;
pub const FEATURES_HEADER: &str = "X-PC-Remote-Features";

/// Extensions requested in the WebSocket upgrade. The header grants nothing by itself.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Features {
    pub input: bool,
    /// files.v1, also implied by files.v2.
    pub files: bool,
    /// Any file type up to 1 GiB and PC → phone pushes.
    pub files_v2: bool,
    pub search: bool,
    pub info: bool,
    /// power.v1: lock, sleep, shut down and restart the PC.
    pub power: bool,
    /// clipboard.v2 on top of files.v2, its transport: images in the clipboard.
    pub clipboard_image: bool,
    /// wol.v1: the PC shares its active MAC addresses in `welcome` so the phone can wake it.
    pub wol: bool,
    /// numpad.v1: dedicated numeric keypad keys (`numpad0`..`numpad9`, operators, `numpad_enter`).
    pub numpad: bool,
    /// presentation.v1: slide navigation, start/stop and screen blackout/whiteout.
    pub presentation: bool,
    /// deck.v1: curated cross-platform quick actions (window management, screenshot, system tools).
    pub deck: bool,
}

impl Features {
    /// Reads every header line and every comma or whitespace separated token.
    pub fn parse<'a>(values: impl IntoIterator<Item = &'a [u8]>) -> Self {
        let mut features = Self::default();
        let mut clipboard_v2 = false;
        for value in values {
            let Ok(text) = std::str::from_utf8(value) else {
                continue;
            };
            for token in text.split(|c: char| c == ',' || c.is_whitespace()) {
                match token.trim() {
                    "input.v1" => features.input = true,
                    "files.v1" => features.files = true,
                    "files.v2" => {
                        features.files = true;
                        features.files_v2 = true;
                    }
                    "search.v1" => features.search = true,
                    "info.v1" => features.info = true,
                    "power.v1" => features.power = true,
                    "clipboard.v2" => clipboard_v2 = true,
                    "wol.v1" => features.wol = true,
                    "numpad.v1" => features.numpad = true,
                    "presentation.v1" => features.presentation = true,
                    "deck.v1" => features.deck = true,
                    _ => {}
                }
            }
        }
        features.clipboard_image = clipboard_v2 && features.files_v2;
        features
    }

    /// Tests: the extensions of the 0.2–0.5 clients.
    #[cfg(test)]
    pub fn legacy(input: bool, files: bool) -> Self {
        Self {
            input,
            files,
            ..Self::default()
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum ClientMessage {
    #[serde(rename = "pair.request")]
    PairRequest {
        v: u8,
        id: Uuid,
        #[serde(rename = "deviceId")]
        device_id: Uuid,
        #[serde(rename = "deviceName")]
        device_name: String,
        code: String,
    },
    #[serde(rename = "hello")]
    Hello {
        v: u8,
        id: Uuid,
        #[serde(rename = "deviceId")]
        device_id: Uuid,
        token: String,
    },
    #[serde(rename = "request")]
    Request { v: u8, id: Uuid, action: Action },
}

impl ClientMessage {
    pub fn version(&self) -> u8 {
        match self {
            Self::PairRequest { v, .. } | Self::Hello { v, .. } | Self::Request { v, .. } => *v,
        }
    }
    pub fn id(&self) -> Uuid {
        match self {
            Self::PairRequest { id, .. } | Self::Hello { id, .. } | Self::Request { id, .. } => *id,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum Action {
    #[serde(rename = "state.get")]
    StateGet,
    #[serde(rename = "volume.set")]
    VolumeSet { percent: u8 },
    #[serde(rename = "volume.step")]
    VolumeStep { delta: i8 },
    #[serde(rename = "volume.mute")]
    VolumeMute { muted: bool },
    #[serde(rename = "media.control")]
    MediaControl {
        #[serde(rename = "sessionId")]
        session_id: String,
        command: MediaCommand,
        #[serde(rename = "offsetSeconds", default)]
        offset_seconds: Option<i8>,
    },
    #[serde(rename = "clipboard.set")]
    ClipboardSet {
        #[serde(rename = "eventId")]
        event_id: Uuid,
        origin: ClipboardOrigin,
        text: String,
        automatic: bool,
        #[serde(rename = "baseRevision")]
        base_revision: u64,
    },
    #[serde(rename = "clipboard.get")]
    ClipboardGet,
    #[serde(rename = "sync.set")]
    SyncSet {
        upload: bool,
        download: bool,
        paused: bool,
    },
    #[serde(rename = "input.enable")]
    InputEnable { enabled: bool },
    #[serde(rename = "pointer.move")]
    PointerMove { dx: i16, dy: i16 },
    #[serde(rename = "pointer.click")]
    PointerClick { button: PointerButton, count: u8 },
    #[serde(rename = "pointer.scroll")]
    PointerScroll { steps: i8 },
    #[serde(rename = "keyboard.text")]
    KeyboardText { text: String },
    #[serde(rename = "keyboard.key")]
    KeyboardKey { key: KeyboardKey },
    /// presentation.v1: slide navigation, start/stop and screen blackout/whiteout.
    #[serde(rename = "presentation.control")]
    PresentationControl { command: PresentationCommand },
    /// deck.v1: curated quick actions (window management, screenshot, system tools).
    #[serde(rename = "deck.trigger")]
    DeckTrigger { id: DeckAction },
    #[serde(rename = "file.offer")]
    FileOffer {
        #[serde(rename = "transferId")]
        transfer_id: Uuid,
        name: String,
        size: u64,
        mime: String,
        /// clipboard.v2: the image goes to the Windows clipboard instead of Downloads.
        #[serde(default)]
        clipboard: Option<ClipboardOffer>,
    },
    #[serde(rename = "file.cancel")]
    FileCancel {
        #[serde(rename = "transferId")]
        transfer_id: Uuid,
    },
    /// files.v2: the phone refuses a `file.push` it cannot store.
    #[serde(rename = "file.decline")]
    FileDecline {
        #[serde(rename = "transferId")]
        transfer_id: Uuid,
        code: DeclineCode,
    },
    /// search.v1: the PC opens a search page with a fixed origin.
    #[serde(rename = "search.open")]
    SearchOpen {
        service: SearchService,
        query: String,
    },
    /// power.v1: now (0) or scheduled (1..86400 s); one schedule per PC.
    #[serde(rename = "power.run")]
    PowerRun {
        action: PowerAction,
        #[serde(rename = "delaySeconds")]
        delay_seconds: u32,
    },
    /// power.v1: drops the schedule, if any. A struct variant so extra fields are refused.
    #[serde(rename = "power.cancel")]
    PowerCancel {},
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PowerAction {
    Lock,
    Sleep,
    Shutdown,
    Restart,
}

/// Longest `power.run` delay: 24 h.
pub const MAX_POWER_DELAY: u32 = 86_400;

/// clipboard.v2 fields of a `file.offer`, with the rules of `clipboard.set`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClipboardOffer {
    #[serde(rename = "eventId")]
    pub event_id: Uuid,
    pub automatic: bool,
    #[serde(rename = "baseRevision")]
    pub base_revision: u64,
}

/// clipboard.v2 image limits, in both directions.
pub const MAX_IMAGE_BYTES: u64 = 33_554_432;
pub const MAX_IMAGE_PIXELS: u64 = 50_000_000;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DeclineCode {
    Storage,
    Unavailable,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SearchService {
    Youtube,
}

pub const MAX_SEARCH_CHARS: usize = 200;

/// Text messages accepted on `/v1/files`. Binary frames carry the file bytes:
/// from the phone after `file.begin`, from the PC after `file.fetch` (files.v2).
#[derive(Debug, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum FileClientMessage {
    #[serde(rename = "file.begin")]
    Begin {
        v: u8,
        id: Uuid,
        #[serde(rename = "transferId")]
        transfer_id: Uuid,
        ticket: String,
    },
    #[serde(rename = "file.end")]
    End {
        v: u8,
        id: Uuid,
        #[serde(rename = "transferId")]
        transfer_id: Uuid,
        sha256: String,
    },
    #[serde(rename = "file.fetch")]
    Fetch {
        v: u8,
        id: Uuid,
        #[serde(rename = "transferId")]
        transfer_id: Uuid,
        ticket: String,
    },
    /// Bytes the phone wrote; the only message without an id.
    #[serde(rename = "file.progress")]
    Progress {
        v: u8,
        #[serde(rename = "transferId")]
        transfer_id: Uuid,
        received: u64,
    },
    #[serde(rename = "file.done")]
    Done {
        v: u8,
        id: Uuid,
        #[serde(rename = "transferId")]
        transfer_id: Uuid,
        name: String,
    },
    #[serde(rename = "error")]
    Error {
        v: u8,
        id: Uuid,
        code: String,
        /// Parsed for strictness; the PC shows its own wording.
        #[allow(dead_code)]
        message: String,
        #[allow(dead_code)]
        retryable: bool,
    },
}

impl FileClientMessage {
    pub fn version(&self) -> u8 {
        match self {
            Self::Begin { v, .. }
            | Self::End { v, .. }
            | Self::Fetch { v, .. }
            | Self::Progress { v, .. }
            | Self::Done { v, .. }
            | Self::Error { v, .. } => *v,
        }
    }
    /// `file.progress` has no id and reports the nil UUID.
    pub fn id(&self) -> Uuid {
        match self {
            Self::Begin { id, .. }
            | Self::End { id, .. }
            | Self::Fetch { id, .. }
            | Self::Done { id, .. }
            | Self::Error { id, .. } => *id,
            Self::Progress { .. } => Uuid::nil(),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PointerButton {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum KeyboardKey {
    Enter,
    Backspace,
    Tab,
    Escape,
    Left,
    Right,
    Up,
    Down,
    Delete,
    Home,
    End,
    Numpad0,
    Numpad1,
    Numpad2,
    Numpad3,
    Numpad4,
    Numpad5,
    Numpad6,
    Numpad7,
    Numpad8,
    Numpad9,
    #[serde(rename = "numpad_decimal")]
    NumpadDecimal,
    #[serde(rename = "numpad_comma")]
    NumpadComma,
    #[serde(rename = "numpad_add")]
    NumpadAdd,
    #[serde(rename = "numpad_subtract")]
    NumpadSubtract,
    #[serde(rename = "numpad_multiply")]
    NumpadMultiply,
    #[serde(rename = "numpad_divide")]
    NumpadDivide,
    #[serde(rename = "numpad_enter")]
    NumpadEnter,
}

impl KeyboardKey {
    /// True for keys added by `numpad.v1`.
    pub fn is_numpad(self) -> bool {
        matches!(
            self,
            Self::Numpad0
                | Self::Numpad1
                | Self::Numpad2
                | Self::Numpad3
                | Self::Numpad4
                | Self::Numpad5
                | Self::Numpad6
                | Self::Numpad7
                | Self::Numpad8
                | Self::Numpad9
                | Self::NumpadDecimal
                | Self::NumpadComma
                | Self::NumpadAdd
                | Self::NumpadSubtract
                | Self::NumpadMultiply
                | Self::NumpadDivide
                | Self::NumpadEnter
        )
    }
}

/// presentation.v1 commands mapped to standard presentation shortcuts on Windows and Linux.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PresentationCommand {
    Start,
    StartCurrent,
    Stop,
    Next,
    Previous,
    Blackout,
    Whiteout,
}

/// deck.v1 curated cross-platform system and window actions.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeckAction {
    ShowDesktop,
    TaskView,
    SwitchWindow,
    CloseWindow,
    Screenshot,
    Fullscreen,
    LockPc,
    Calculator,
    Downloads,
    Browser,
    TaskManager,
    EmojiPicker,
}

impl Action {
    pub fn is_input(&self) -> bool {
        matches!(
            self,
            Self::InputEnable { .. }
                | Self::PointerMove { .. }
                | Self::PointerClick { .. }
                | Self::PointerScroll { .. }
                | Self::KeyboardText { .. }
                | Self::KeyboardKey { .. }
                | Self::PresentationControl { .. }
                | Self::DeckTrigger { .. }
        )
    }
    pub fn is_file(&self) -> bool {
        matches!(
            self,
            Self::FileOffer { .. } | Self::FileCancel { .. } | Self::FileDecline { .. }
        )
    }
    pub fn is_power(&self) -> bool {
        matches!(self, Self::PowerRun { .. } | Self::PowerCancel {})
    }
    /// A clipboard.v2 image offer, which reports the image limits on errors.
    pub fn is_clipboard_image(&self) -> bool {
        matches!(
            self,
            Self::FileOffer {
                clipboard: Some(_),
                ..
            }
        )
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ClipboardOrigin {
    Android,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaCommand {
    Play,
    Pause,
    Previous,
    Next,
    Seek,
}

/// Field rules that do not depend on the connection: files.v1 offers and every other action.
#[cfg(test)]
pub fn validate_action(action: &Action) -> Result<(), &'static str> {
    validate_action_for(action, &Features::default())
}

/// Same rules, with `file.offer` checked against what the connection negotiated:
/// files.v2 for any file, clipboard.v2 for the `clipboard` field, plus numpad/presentation/deck.
pub fn validate_action_for(action: &Action, negotiated: &Features) -> Result<(), &'static str> {
    match action {
        Action::VolumeSet { percent } if *percent > 100 => Err("invalid_message"),
        Action::VolumeStep { delta } if *delta != -5 && *delta != 5 => Err("invalid_message"),
        Action::MediaControl {
            session_id,
            command,
            offset_seconds,
        } => {
            if session_id.is_empty() || session_id.len() > 256 {
                return Err("invalid_message");
            }
            if matches!(command, MediaCommand::Seek) != offset_seconds.is_some() {
                return Err("invalid_message");
            }
            if let Some(offset) = offset_seconds {
                if *offset != -10 && *offset != 10 {
                    return Err("invalid_message");
                }
            }
            Ok(())
        }
        Action::ClipboardSet { text, .. } if text.as_bytes().len() > MAX_TEXT => Err("too_large"),
        Action::ClipboardSet { text, .. } if text.contains('\0') => Err("invalid_message"),
        Action::PointerMove { dx, dy }
            if (*dx == 0 && *dy == 0) || dx.unsigned_abs() > 512 || dy.unsigned_abs() > 512 =>
        {
            Err("invalid_message")
        }
        Action::PointerClick { button, count }
            if *count == 0
                || *count > 2
                || (matches!(button, PointerButton::Right) && *count != 1) =>
        {
            Err("invalid_message")
        }
        Action::PointerScroll { steps } if *steps == 0 || steps.unsigned_abs() > 10 => {
            Err("invalid_message")
        }
        Action::KeyboardText { text }
            if text.is_empty() || text.len() > 1024 || text.chars().any(char::is_control) =>
        {
            Err("invalid_message")
        }
        Action::KeyboardKey { key } if key.is_numpad() && !negotiated.numpad => Err("unavailable"),
        Action::PresentationControl { .. } if !negotiated.presentation => Err("unavailable"),
        Action::DeckTrigger { .. } if !negotiated.deck => Err("unavailable"),
        Action::FileOffer {
            clipboard: Some(clipboard),
            ..
        } if !negotiated.clipboard_image || clipboard.event_id.is_nil() => Err("invalid_message"),
        Action::FileOffer {
            transfer_id,
            name,
            size,
            mime,
            clipboard: Some(_),
        } => crate::files::check_clipboard_offer(*transfer_id, name, *size, mime).map(|_| ()),
        Action::FileOffer {
            transfer_id,
            name,
            size,
            mime,
            ..
        } if negotiated.files_v2 => {
            crate::files::check_offer_v2(*transfer_id, name, *size, mime).map(|_| ())
        }
        Action::FileOffer {
            transfer_id,
            name,
            size,
            mime,
            ..
        } => crate::files::check_offer(*transfer_id, name, *size, mime).map(|_| ()),
        Action::FileCancel { transfer_id } | Action::FileDecline { transfer_id, .. }
            if transfer_id.is_nil() =>
        {
            Err("invalid_message")
        }
        Action::SearchOpen { query, .. } => search_query(query).map(|_| ()),
        Action::PowerRun {
            action,
            delay_seconds,
        } if *delay_seconds > MAX_POWER_DELAY
            || (*action == PowerAction::Lock && *delay_seconds != 0) =>
        {
            Err("invalid_message")
        }
        _ => Ok(()),
    }
}

/// The trimmed query of `search.open`: 1..200 characters, no controls, not only spaces.
pub fn search_query(raw: &str) -> Result<&str, &'static str> {
    let query = raw.trim();
    if query.is_empty()
        || raw.chars().count() > MAX_SEARCH_CHARS
        || raw.chars().any(char::is_control)
    {
        return Err("invalid_message");
    }
    Ok(query)
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioState {
    pub available: bool,
    pub percent: u8,
    pub muted: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct MediaCapabilities {
    pub play: bool,
    pub pause: bool,
    pub previous: bool,
    pub next: bool,
    pub seek: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MediaSession {
    pub id: String,
    pub app: String,
    pub title: String,
    pub playing: bool,
    pub capabilities: MediaCapabilities,
    #[serde(rename = "positionSeconds")]
    pub position_seconds: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MediaState {
    pub sessions: Vec<MediaSession>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SyncSettings {
    pub upload: bool,
    pub download: bool,
    pub paused: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct StateSnapshot {
    pub v: u8,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub revision: u64,
    pub audio: AudioState,
    pub media: MediaState,
    #[serde(rename = "clipboardRevision")]
    pub clipboard_revision: u64,
    pub sync: SyncSettings,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<InputState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<FilesState>,
    /// power.v1 only; older phones refuse unknown fields.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power: Option<PowerState>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PowerState {
    /// False while "Interromper" blocks the phone's control of the PC.
    pub allowed: bool,
    /// Serialized as `null` when nothing is scheduled.
    pub scheduled: Option<ScheduledPower>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ScheduledPower {
    pub action: PowerAction,
    /// 1..86400, rounded up when the snapshot was built.
    #[serde(rename = "remainingSeconds")]
    pub remaining_seconds: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct InputState {
    pub allowed: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FilesState {
    pub allowed: bool,
    #[serde(rename = "maxBytes")]
    pub max_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClipboardEvent {
    pub v: u8,
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(rename = "eventId")]
    pub event_id: Uuid,
    pub origin: &'static str,
    pub revision: u64,
    pub text: String,
    pub automatic: bool,
}

pub fn wire_error(id: Uuid, code: &str, message: &str, retryable: bool) -> serde_json::Value {
    serde_json::json!({"v":1,"type":"error","id":id,"code":code,"message":message,"retryable":retryable})
}
pub fn ack(id: Uuid) -> serde_json::Value {
    serde_json::json!({"v":1,"type":"ack","id":id})
}

/// Which limit a `too_large` error talks about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wording {
    Control,
    FilesV1,
    FilesV2,
    /// clipboard.v2 images.
    Image,
}

/// pt-BR message and retry hint for an error code. File actions report their own size limit.
pub fn error_text(code: &str, wording: Wording) -> (&'static str, bool) {
    match code {
        "version_mismatch" => ("Versão do protocolo incompatível", false),
        "unauthorized" => ("Dispositivo sem autorização", false),
        "pairing_code_incorrect" => ("Código incorreto para este PC", false),
        "pairing_expired" => ("Código expirado. Gere outro no PC", false),
        "pairing_timeout" => ("Solicitação de pareamento expirada", false),
        "pairing_invalidated" => ("Solicitação substituída por um novo código", false),
        "pairing_denied" => ("Pareamento recusado no PC", false),
        "pairing_failed" => ("Não foi possível salvar o pareamento no PC", true),
        "rate_limited" => ("Muitas tentativas. Tente depois.", true),
        "unavailable" => ("Ação indisponível neste dispositivo ou player", false),
        "conflict" => ("Houve outra cópia antes desta", false),
        "sensitive" => ("Conteúdo marcado como sensível", false),
        "too_large" if wording == Wording::FilesV2 => ("Arquivo excede 1 GB", false),
        "too_large" if wording == Wording::FilesV1 => ("Arquivo excede 50 MB", false),
        "too_large" if wording == Wording::Image => ("Imagem excede 32 MB", false),
        "too_large" => ("Texto excede 64 KiB", false),
        "clipboard_busy" => ("Área de transferência ocupada", true),
        "invalid_message" => ("Mensagem inválida", false),
        "file_type" => ("Tipo de arquivo não permitido", false),
        "file_busy" => ("Outro arquivo está sendo recebido", true),
        "file_integrity" => ("Arquivo corrompido durante o envio", true),
        "storage" => ("Não foi possível salvar o arquivo no PC", true),
        _ => ("Falha ao executar ação", true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unknown_command_fields() {
        let x = serde_json::from_str::<ClientMessage>(
            r#"{"v":1,"type":"request","id":"00000000-0000-0000-0000-000000000001","action":{"kind":"volume.set","percent":5,"shell":"cmd"}}"#,
        );
        assert!(x.is_err());
    }
    #[test]
    fn checks_bounds_and_seek() {
        assert_eq!(
            validate_action(&Action::VolumeSet { percent: 101 }),
            Err("invalid_message")
        );
        assert_eq!(
            validate_action(&Action::MediaControl {
                session_id: "x".into(),
                command: MediaCommand::Seek,
                offset_seconds: None
            }),
            Err("invalid_message")
        );
        assert_eq!(
            validate_action(&Action::ClipboardSet {
                event_id: Uuid::new_v4(),
                origin: ClipboardOrigin::Android,
                text: "x".repeat(MAX_TEXT + 1),
                automatic: false,
                base_revision: 0
            }),
            Err("too_large")
        );
    }
    #[test]
    fn shared_golden_examples() {
        let examples: serde_json::Value =
            serde_json::from_str(include_str!("../../docs/protocol-v1.examples.json")).unwrap();
        assert_eq!(examples["protocolVersion"], 1);
        for raw in examples["client"].as_array().unwrap() {
            let msg: ClientMessage = serde_json::from_value(raw.clone()).unwrap();
            assert_eq!(msg.version(), 1);
            if let ClientMessage::Request { action, .. } = msg {
                validate_action(&action).unwrap();
            }
        }
        for raw in examples["invalidClient"].as_array().unwrap() {
            if let Ok(ClientMessage::Request { action, .. }) =
                serde_json::from_value::<ClientMessage>(raw.clone())
            {
                assert!(validate_action(&action).is_err());
            }
        }
        for raw in examples["fileSocket"]["client"].as_array().unwrap() {
            let msg: FileClientMessage = serde_json::from_value(raw.clone()).unwrap();
            assert_eq!(msg.version(), 1);
            assert!(raw.to_string().len() <= FILE_TEXT_LIMIT);
        }
        for raw in examples["fileSocket"]["invalidClient"].as_array().unwrap() {
            assert!(serde_json::from_value::<FileClientMessage>(raw.clone()).is_err());
        }
    }
    #[test]
    fn negotiated_06_golden_examples() {
        let examples: serde_json::Value =
            serde_json::from_str(include_str!("../../docs/protocol-v1.examples.json")).unwrap();
        let section = &examples["negotiated06"];
        let headers: Vec<&[u8]> = section["headers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h.as_str().unwrap().as_bytes())
            .collect();
        assert_eq!(
            Features::parse(headers),
            Features {
                input: true,
                files: true,
                files_v2: true,
                search: true,
                info: true,
                ..Features::default()
            }
        );
        let negotiated = Features::parse(
            section["headers"]
                .as_array()
                .unwrap()
                .iter()
                .map(|h| h.as_str().unwrap().as_bytes()),
        );
        for raw in section["client"].as_array().unwrap() {
            let ClientMessage::Request { action, .. } =
                serde_json::from_value::<ClientMessage>(raw.clone()).unwrap()
            else {
                panic!("{raw}")
            };
            validate_action_for(&action, &negotiated)
                .unwrap_or_else(|code| panic!("{code}: {raw}"));
        }
        for raw in section["invalidClient"].as_array().unwrap() {
            let invalid = match serde_json::from_value::<ClientMessage>(raw.clone()) {
                Ok(ClientMessage::Request { action, .. }) => {
                    validate_action_for(&action, &negotiated).is_err()
                }
                Ok(_) => false,
                Err(_) => true,
            };
            assert!(invalid, "{raw}");
        }
        for raw in section["fileSocket"]["client"].as_array().unwrap() {
            let message: FileClientMessage = serde_json::from_value(raw.clone()).unwrap();
            assert_eq!(message.version(), 1);
        }
        for raw in section["fileSocket"]["invalidClient"].as_array().unwrap() {
            assert!(
                serde_json::from_value::<FileClientMessage>(raw.clone()).is_err(),
                "{raw}"
            );
        }
        // The v1 rules still refuse what only files.v2 accepts.
        let video = &section["client"][0]["action"];
        let action: Action = serde_json::from_value(video.clone()).unwrap();
        assert_eq!(
            validate_action_for(&action, &Features::default()),
            Err("too_large")
        );
    }

    #[test]
    fn negotiated_061_golden_examples() {
        let examples: serde_json::Value =
            serde_json::from_str(include_str!("../../docs/protocol-v1.examples.json")).unwrap();
        let section = &examples["negotiated061"];
        let negotiated = Features::parse(
            section["headers"]
                .as_array()
                .unwrap()
                .iter()
                .map(|h| h.as_str().unwrap().as_bytes()),
        );
        assert_eq!(
            negotiated,
            Features {
                input: true,
                files: true,
                files_v2: true,
                search: true,
                info: true,
                power: true,
                clipboard_image: true,
                ..Features::default()
            }
        );
        let without_clipboard = Features {
            clipboard_image: false,
            ..negotiated
        };
        let mut image_offers = 0;
        for raw in section["client"].as_array().unwrap() {
            let ClientMessage::Request { action, .. } =
                serde_json::from_value::<ClientMessage>(raw.clone()).unwrap()
            else {
                panic!("{raw}")
            };
            validate_action_for(&action, &negotiated)
                .unwrap_or_else(|code| panic!("{code}: {raw}"));
            // The clipboard field exists only on clipboard.v2 connections.
            if action.is_clipboard_image() {
                image_offers += 1;
                assert_eq!(
                    validate_action_for(&action, &without_clipboard),
                    Err("invalid_message"),
                    "{raw}"
                );
            }
        }
        assert_eq!(image_offers, 2);
        let mut codes = Vec::new();
        for raw in section["invalidClient"].as_array().unwrap() {
            match serde_json::from_value::<ClientMessage>(raw.clone()) {
                Ok(ClientMessage::Request { action, .. }) => {
                    codes.push(validate_action_for(&action, &negotiated).unwrap_err())
                }
                Ok(_) => panic!("{raw}"),
                Err(_) => codes.push("parse"),
            }
        }
        assert_eq!(
            codes,
            [
                "invalid_message",
                "invalid_message",
                "parse",
                "parse",
                "parse",
                "parse",
                "file_type",
                "too_large",
                "parse",
                "parse"
            ]
        );
        // Only negotiated headers count: clipboard.v2 needs files.v2, its transport.
        let lone = Features::parse([b"clipboard.v2".as_slice(), b"files.v1"]);
        assert!(!lone.clipboard_image);
        let with_files = Features::parse([b"files.v2, clipboard.v2".as_slice()]);
        assert!(with_files.clipboard_image && with_files.files_v2);
        assert!(Features::parse([b"power.v1".as_slice()]).power);
    }

    #[test]
    fn wol_numpad_presentation_and_deck_require_negotiation() {
        let negotiated =
            Features::parse([b"input.v1, wol.v1, numpad.v1, presentation.v1, deck.v1".as_slice()]);
        assert!(
            negotiated.input
                && negotiated.wol
                && negotiated.numpad
                && negotiated.presentation
                && negotiated.deck
        );
        let numpad = Action::KeyboardKey {
            key: KeyboardKey::Numpad5,
        };
        let pres = Action::PresentationControl {
            command: PresentationCommand::Next,
        };
        let deck = Action::DeckTrigger {
            id: DeckAction::ShowDesktop,
        };
        assert_eq!(validate_action(&numpad), Err("unavailable"));
        assert_eq!(validate_action(&pres), Err("unavailable"));
        assert_eq!(validate_action(&deck), Err("unavailable"));
        assert_eq!(validate_action_for(&numpad, &negotiated), Ok(()));
        assert_eq!(validate_action_for(&pres, &negotiated), Ok(()));
        assert_eq!(validate_action_for(&deck, &negotiated), Ok(()));
    }

    #[test]
    fn power_state_serializes_null_and_remaining_seconds() {
        let idle = PowerState {
            allowed: true,
            scheduled: None,
        };
        assert_eq!(
            serde_json::to_value(&idle).unwrap(),
            serde_json::json!({"allowed":true,"scheduled":null})
        );
        let scheduled = PowerState {
            allowed: false,
            scheduled: Some(ScheduledPower {
                action: PowerAction::Shutdown,
                remaining_seconds: 1740,
            }),
        };
        assert_eq!(
            serde_json::to_value(&scheduled).unwrap(),
            serde_json::json!({"allowed":false,"scheduled":{"action":"shutdown","remainingSeconds":1740}})
        );
        assert_eq!(
            error_text("too_large", Wording::Image),
            ("Imagem excede 32 MB", false)
        );
    }

    #[test]
    fn search_queries_are_trimmed_bounded_and_without_controls() {
        assert_eq!(search_query("  gatos  "), Ok("gatos"));
        assert_eq!(search_query(&"é".repeat(200)), Ok("é".repeat(200).as_str()));
        for bad in ["", "   ", "a\nb", "a\u{7}", &"x".repeat(201)] {
            assert_eq!(search_query(bad), Err("invalid_message"), "{bad:?}");
        }
    }

    #[test]
    fn file_offer_rules_and_strict_file_socket_messages() {
        let offer = |name: &str, size: u64, mime: &str| Action::FileOffer {
            transfer_id: Uuid::new_v4(),
            name: name.into(),
            size,
            mime: mime.into(),
            clipboard: None,
        };
        assert_eq!(validate_action(&offer("a.png", 1, "image/png")), Ok(()));
        assert_eq!(
            validate_action(&offer("a.pdf", MAX_FILE_BYTES, "application/pdf")),
            Ok(())
        );
        let longest = format!("{}.jpg", "é".repeat(125));
        assert_eq!(longest.len(), 254);
        assert_eq!(validate_action(&offer(&longest, 1, "image/jpeg")), Ok(()));
        for (name, size, mime, code) in [
            ("a.png", 0, "image/png", "invalid_message"),
            ("a.png", MAX_FILE_BYTES + 1, "image/png", "too_large"),
            ("", 1, "image/png", "invalid_message"),
            ("a\n.png", 1, "image/png", "invalid_message"),
            ("a\0.png", 1, "image/png", "invalid_message"),
            ("a.png", 1, "image/jpeg", "file_type"),
            ("a.gif", 1, "image/gif", "file_type"),
        ] {
            assert_eq!(
                validate_action(&offer(name, size, mime)),
                Err(code),
                "{name}"
            );
        }
        let too_long = format!("{}.png", "a".repeat(252));
        assert_eq!(
            validate_action(&offer(&too_long, 1, "image/png")),
            Err("invalid_message")
        );
        assert_eq!(
            validate_action(&Action::FileOffer {
                transfer_id: Uuid::nil(),
                name: "a.png".into(),
                size: 1,
                mime: "image/png".into(),
                clipboard: None,
            }),
            Err("invalid_message")
        );
        assert_eq!(
            validate_action(&Action::FileCancel {
                transfer_id: Uuid::nil()
            }),
            Err("invalid_message")
        );
        let request = |action: serde_json::Value| {
            serde_json::from_value::<ClientMessage>(
                serde_json::json!({"v":1,"type":"request","id":Uuid::new_v4(),"action":action}),
            )
        };
        let id = Uuid::new_v4();
        for size in [
            serde_json::json!(-1),
            serde_json::json!(1.5),
            serde_json::json!("10"),
        ] {
            assert!(request(serde_json::json!({"kind":"file.offer","transferId":id,"name":"a.png","size":size,"mime":"image/png"})).is_err());
        }
        assert!(request(serde_json::json!({"kind":"file.offer","transferId":id,"name":"a.png","size":1,"mime":"image/png","path":"C:\\"})).is_err());
        assert!(
            request(serde_json::json!({"kind":"file.cancel","transferId":id,"force":true}))
                .is_err()
        );
        let file = |value: serde_json::Value| serde_json::from_value::<FileClientMessage>(value);
        assert!(file(
            serde_json::json!({"v":1,"type":"file.begin","id":id,"transferId":id,"ticket":"ab"})
        )
        .is_ok());
        assert!(file(serde_json::json!({"v":1,"type":"file.begin","id":id,"transferId":id,"ticket":"ab","name":"x"})).is_err());
        assert!(
            file(serde_json::json!({"v":1,"type":"file.end","id":id,"transferId":id})).is_err()
        );
        assert!(
            file(serde_json::json!({"v":1,"type":"file.chunk","id":id,"transferId":id})).is_err()
        );
        assert!(
            file(serde_json::json!({"v":1,"type":"hello","id":id,"deviceId":id,"token":"t"}))
                .is_err()
        );
    }
    #[test]
    fn file_errors_use_file_wording_without_changing_clipboard_messages() {
        assert_eq!(
            error_text("too_large", Wording::Control),
            ("Texto excede 64 KiB", false)
        );
        assert_eq!(
            error_text("too_large", Wording::FilesV1),
            ("Arquivo excede 50 MB", false)
        );
        assert_eq!(
            error_text("too_large", Wording::FilesV2),
            ("Arquivo excede 1 GB", false)
        );
        assert_eq!(
            error_text("file_type", Wording::FilesV1),
            ("Tipo de arquivo não permitido", false)
        );
        assert_eq!(
            error_text("file_busy", Wording::FilesV2),
            ("Outro arquivo está sendo recebido", true)
        );
        assert_eq!(
            error_text("file_integrity", Wording::FilesV1),
            ("Arquivo corrompido durante o envio", true)
        );
        assert_eq!(
            error_text("storage", Wording::FilesV1),
            ("Não foi possível salvar o arquivo no PC", true)
        );
        assert_eq!(
            error_text("internal", Wording::FilesV2),
            ("Falha ao executar ação", true)
        );
    }
    #[test]
    fn escaped_unicode_frame_fits_limit() {
        let value = serde_json::json!({"v":1,"type":"request","id":Uuid::new_v4(),"action":{"kind":"clipboard.set","eventId":Uuid::new_v4(),"origin":"android","text":"\u{001f}".repeat(MAX_TEXT),"automatic":false,"baseRevision":0}});
        assert!(value.to_string().len() <= MAX_FRAME);
    }
    #[test]
    fn input_validation_rejects_oversized_motion_text_controls_and_unknown_keys() {
        for action in [
            Action::PointerMove {
                dx: i16::MIN,
                dy: 0,
            },
            Action::PointerMove { dx: 0, dy: 0 },
            Action::PointerScroll { steps: i8::MIN },
            Action::PointerClick {
                button: PointerButton::Right,
                count: 2,
            },
            Action::KeyboardText {
                text: "abc\n".into(),
            },
            Action::KeyboardText {
                text: "a".repeat(1025),
            },
        ] {
            assert!(validate_action(&action).is_err());
        }
        let value = serde_json::json!({"v":1,"type":"request","id":Uuid::new_v4(),"action":{"kind":"keyboard.key","key":"power"}});
        assert!(serde_json::from_value::<ClientMessage>(value).is_err());
        assert_eq!(
            validate_action(&Action::KeyboardText {
                text: "á🦊".into()
            }),
            Ok(())
        );
    }
}
