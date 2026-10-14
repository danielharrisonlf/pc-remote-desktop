use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const PORT: u16 = 43127;
pub const MAX_FRAME: usize = 512 * 1024;
pub const MAX_TEXT: usize = 64 * 1024;

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
    }
    #[test]
    fn escaped_unicode_frame_fits_limit() {
        let value = serde_json::json!({"v":1,"type":"request","id":Uuid::new_v4(),"action":{"kind":"clipboard.set","eventId":Uuid::new_v4(),"origin":"android","text":"\u{001f}".repeat(MAX_TEXT),"automatic":false,"baseRevision":0}});
        assert!(value.to_string().len() <= MAX_FRAME);
    }
}
