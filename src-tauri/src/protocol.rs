use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const PORT: u16 = 43127;
pub const MAX_FRAME: usize = 512 * 1024;
pub const MAX_TEXT: usize = 64 * 1024;
pub const MAX_FILE_BYTES: u64 = 52_428_800;
pub const FILE_CHUNK_BYTES: usize = 65_536;
pub const FILE_WINDOW_BYTES: usize = 1_048_576;
pub const FILE_TEXT_LIMIT: usize = 4_096;
pub const FEATURES_HEADER: &str = "X-PC-Remote-Features";

/// Extensions requested in the WebSocket upgrade. The header grants nothing by itself.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Features {
    pub input: bool,
    pub files: bool,
}

impl Features {
    /// Reads every header line and every comma or whitespace separated token.
    pub fn parse<'a>(values: impl IntoIterator<Item = &'a [u8]>) -> Self {
        let mut features = Self::default();
        for value in values {
            let Ok(text) = std::str::from_utf8(value) else {
                continue;
            };
            for token in text.split(|c: char| c == ',' || c.is_whitespace()) {
                match token.trim() {
                    "input.v1" => features.input = true,
                    "files.v1" => features.files = true,
                    _ => {}
                }
            }
        }
        features
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
    #[serde(rename = "file.offer")]
    FileOffer {
        #[serde(rename = "transferId")]
        transfer_id: Uuid,
        name: String,
        size: u64,
        mime: String,
    },
    #[serde(rename = "file.cancel")]
    FileCancel {
        #[serde(rename = "transferId")]
        transfer_id: Uuid,
    },
}

/// Text messages accepted on `/v1/files`. Binary frames carry the file bytes.
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
}

impl FileClientMessage {
    pub fn version(&self) -> u8 {
        match self {
            Self::Begin { v, .. } | Self::End { v, .. } => *v,
        }
    }
    pub fn id(&self) -> Uuid {
        match self {
            Self::Begin { id, .. } | Self::End { id, .. } => *id,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PointerButton {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, Deserialize)]
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
        )
    }
    pub fn is_file(&self) -> bool {
        matches!(self, Self::FileOffer { .. } | Self::FileCancel { .. })
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

pub fn validate_action(action: &Action) -> Result<(), &'static str> {
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
        Action::FileOffer {
            transfer_id,
            name,
            size,
            mime,
        } => crate::files::check_offer(*transfer_id, name, *size, mime).map(|_| ()),
        Action::FileCancel { transfer_id } if transfer_id.is_nil() => Err("invalid_message"),
        _ => Ok(()),
    }
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

/// pt-BR message and retry hint for an error code. File actions report their own size limit.
pub fn error_text(code: &str, file: bool) -> (&'static str, bool) {
    match code {
        "version_mismatch" => ("Versão do protocolo incompatível", false),
        "unauthorized" => ("Dispositivo sem autorização", false),
        "pairing_expired" => ("Código inválido ou expirado", false),
        "pairing_denied" => ("Pareamento recusado no PC", false),
        "rate_limited" => ("Muitas tentativas. Tente depois.", true),
        "unavailable" => ("Ação indisponível neste dispositivo ou player", false),
        "conflict" => ("Houve outra cópia antes desta", false),
        "sensitive" => ("Conteúdo marcado como sensível", false),
        "too_large" if file => ("Arquivo excede 50 MB", false),
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
    fn file_offer_rules_and_strict_file_socket_messages() {
        let offer = |name: &str, size: u64, mime: &str| Action::FileOffer {
            transfer_id: Uuid::new_v4(),
            name: name.into(),
            size,
            mime: mime.into(),
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
                mime: "image/png".into()
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
            error_text("too_large", false),
            ("Texto excede 64 KiB", false)
        );
        assert_eq!(
            error_text("too_large", true),
            ("Arquivo excede 50 MB", false)
        );
        assert_eq!(
            error_text("file_type", true),
            ("Tipo de arquivo não permitido", false)
        );
        assert_eq!(
            error_text("file_busy", true),
            ("Outro arquivo está sendo recebido", true)
        );
        assert_eq!(
            error_text("file_integrity", true),
            ("Arquivo corrompido durante o envio", true)
        );
        assert_eq!(
            error_text("storage", true),
            ("Não foi possível salvar o arquivo no PC", true)
        );
        assert_eq!(
            error_text("internal", true),
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
