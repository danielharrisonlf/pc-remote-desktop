//! Linux MPRIS2 D-Bus (`org.mpris.MediaPlayer2.*`) implementation of `MediaBackend`.
//!
//! Communicates over the session D-Bus using `zbus` to inspect active media players, read
//! metadata (`xesam:title`, `xesam:artist`, `xesam:album`, `mpris:length`, `Position`),
//! and dispatch transport controls (`Play`, `Pause`, `PlayPause`, `Previous`, `Next`, `Stop`,
//! `Seek`, `Volume`). Never panics if the session D-Bus is unavailable.
use crate::{
    platform::{friendly_app, MediaBackend, PlatformError},
    protocol::{MediaCapabilities, MediaCommand, MediaSession, MediaState},
};
use std::{
    collections::HashMap,
    sync::{
        mpsc::{self, Sender},
        OnceLock,
    },
    time::Duration,
};
use uuid::Uuid;
use zbus::{
    blocking::{Connection, Proxy},
    zvariant::{OwnedValue, Value},
};

pub struct LinuxMedia;

const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";
const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
const ROOT_IFACE: &str = "org.mpris.MediaPlayer2";
const PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";

enum Task {
    Read(Sender<MediaState>),
    Control(
        String,
        MediaCommand,
        Option<i8>,
        Box<dyn FnOnce() -> bool + Send>,
        Sender<Result<(), PlatformError>>,
    ),
}

struct Entry {
    id: String,
    bus_name: String,
}

#[derive(Default)]
struct Registry {
    conn: Option<Connection>,
    entries: Vec<Entry>,
}

static WORKER: OnceLock<Sender<Task>> = OnceLock::new();

fn worker() -> &'static Sender<Task> {
    WORKER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Task>();
        std::thread::Builder::new()
            .name("pc-remote-media-mpris".into())
            .spawn(move || {
                let mut registry = Registry::default();
                while let Ok(task) = rx.recv() {
                    match task {
                        Task::Read(reply) => {
                            let _ = reply.send(read_inside(&mut registry));
                        }
                        Task::Control(id, command, offset, permit, reply) => {
                            if !permit() {
                                let _ = reply.send(Err(PlatformError::Unauthorized));
                                continue;
                            }
                            let _ = reply.send(
                                control_inside(&mut registry, &id, &command, offset)
                                    .map_err(PlatformError::Unavailable),
                            );
                        }
                    }
                }
            })
            .expect("MPRIS worker thread");
        tx
    })
}

fn connection(registry: &mut Registry) -> Result<Connection, String> {
    if let Some(conn) = &registry.conn {
        return Ok(conn.clone());
    }
    let conn = Connection::session().map_err(|e| format!("D-Bus session unavailable: {e}"))?;
    registry.conn = Some(conn.clone());
    Ok(conn)
}

fn list_mpris_names(conn: &Connection) -> Result<Vec<String>, String> {
    let proxy = Proxy::new(
        conn,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )
    .map_err(|e| e.to_string())?;
    let names: Vec<String> = proxy.call("ListNames", &()).map_err(|e| e.to_string())?;
    Ok(names
        .into_iter()
        .filter(|name| name.starts_with(MPRIS_PREFIX))
        .collect())
}

fn player_proxy<'a>(conn: &'a Connection, bus_name: &'a str) -> Result<Proxy<'a>, String> {
    Proxy::new(conn, bus_name, MPRIS_PATH, PLAYER_IFACE).map_err(|e| e.to_string())
}

fn root_proxy<'a>(conn: &'a Connection, bus_name: &'a str) -> Result<Proxy<'a>, String> {
    Proxy::new(conn, bus_name, MPRIS_PATH, ROOT_IFACE).map_err(|e| e.to_string())
}

fn is_playing(conn: &Connection, bus_name: &str) -> bool {
    player_proxy(conn, bus_name)
        .ok()
        .and_then(|p| p.get_property::<String>("PlaybackStatus").ok())
        .is_some_and(|status| status.eq_ignore_ascii_case("Playing"))
}

fn refresh(registry: &mut Registry) -> Result<Connection, String> {
    let conn = connection(registry)?;
    let names = match list_mpris_names(&conn) {
        Ok(names) => names,
        Err(err) => {
            registry.conn = None;
            return Err(err);
        }
    };

    let mut ranked: Vec<(u8, Entry)> = names
        .into_iter()
        .map(|bus_name| {
            let id = registry
                .entries
                .iter()
                .find(|e| e.bus_name == bus_name)
                .map(|e| e.id.clone())
                .unwrap_or_else(|| Uuid::new_v4().to_string());
            let rank = if is_playing(&conn, &bus_name) { 0 } else { 1 };
            (rank, Entry { id, bus_name })
        })
        .collect();
    ranked.sort_by_key(|(rank, _)| *rank);
    registry.entries = ranked.into_iter().map(|(_, entry)| entry).collect();
    Ok(conn)
}

/// Extracts a string from an MPRIS metadata `OwnedValue` (handles String or Array of Strings).
pub(crate) fn extract_metadata_string(val: &OwnedValue) -> Option<String> {
    if let Ok(s) = <&str>::try_from(val) {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    if let Value::Array(arr) = &**val {
        let joined: Vec<&str> = arr
            .iter()
            .filter_map(|item| <&str>::try_from(item).ok())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if !joined.is_empty() {
            return Some(joined.join(", "));
        }
    }
    None
}

/// Extracts an integer duration/length in microseconds from an MPRIS metadata `OwnedValue`.
pub(crate) fn extract_metadata_micros(val: &OwnedValue) -> Option<i64> {
    if let Ok(v) = i64::try_from(val) {
        return (v > 0).then_some(v);
    }
    if let Ok(v) = u64::try_from(val) {
        return (v > 0 && v <= i64::MAX as u64).then_some(v as i64);
    }
    None
}

fn read_inside(registry: &mut Registry) -> MediaState {
    let Ok(conn) = refresh(registry) else {
        registry.entries.clear();
        return MediaState {
            sessions: Vec::new(),
        };
    };

    let sessions = registry
        .entries
        .iter()
        .filter_map(|entry| {
            let player = player_proxy(&conn, &entry.bus_name).ok()?;
            let root = root_proxy(&conn, &entry.bus_name).ok();

            let raw_identity = root
                .as_ref()
                .and_then(|r| r.get_property::<String>("Identity").ok())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| {
                    entry
                        .bus_name
                        .strip_prefix(MPRIS_PREFIX)
                        .unwrap_or(&entry.bus_name)
                        .to_string()
                });
            let app = friendly_app(&raw_identity);

            let status = player
                .get_property::<String>("PlaybackStatus")
                .unwrap_or_default();
            let playing = status.eq_ignore_ascii_case("Playing");

            let metadata: HashMap<String, OwnedValue> =
                player.get_property("Metadata").unwrap_or_default();
            let title = metadata
                .get("xesam:title")
                .and_then(extract_metadata_string)
                .or_else(|| {
                    metadata
                        .get("xesam:artist")
                        .and_then(extract_metadata_string)
                })
                .or_else(|| {
                    metadata
                        .get("xesam:album")
                        .and_then(extract_metadata_string)
                })
                .unwrap_or_default();

            let length_us = metadata
                .get("mpris:length")
                .and_then(extract_metadata_micros);
            let position_us = player
                .get_property::<i64>("Position")
                .ok()
                .filter(|&p| p >= 0);

            let can_seek = player.get_property::<bool>("CanSeek").unwrap_or(false)
                && (length_us.is_some() || position_us.is_some());

            let caps = MediaCapabilities {
                play: player.get_property::<bool>("CanPlay").unwrap_or(false),
                pause: player.get_property::<bool>("CanPause").unwrap_or(false),
                previous: player
                    .get_property::<bool>("CanGoPrevious")
                    .unwrap_or(false),
                next: player.get_property::<bool>("CanGoNext").unwrap_or(false),
                seek: can_seek,
            };

            Some(MediaSession {
                id: entry.id.clone(),
                app,
                title,
                playing,
                capabilities: caps,
                position_seconds: position_us.map(|us| us as f64 / 1_000_000.0),
            })
        })
        .collect();

    MediaState { sessions }
}

fn control_inside(
    registry: &mut Registry,
    id: &str,
    command: &MediaCommand,
    offset: Option<i8>,
) -> Result<(), String> {
    let conn = refresh(registry)?;
    let bus_name = registry
        .entries
        .iter()
        .find(|e| e.id == id)
        .ok_or("Sessão de mídia indisponível")?
        .bus_name
        .clone();
    let player = player_proxy(&conn, &bus_name)?;

    let allowed = match command {
        MediaCommand::Play => player.get_property::<bool>("CanPlay").unwrap_or(false),
        MediaCommand::Pause => player.get_property::<bool>("CanPause").unwrap_or(false),
        MediaCommand::Previous => player
            .get_property::<bool>("CanGoPrevious")
            .unwrap_or(false),
        MediaCommand::Next => player.get_property::<bool>("CanGoNext").unwrap_or(false),
        MediaCommand::Seek => player.get_property::<bool>("CanSeek").unwrap_or(false),
    };
    if !allowed {
        return Err("Player não permite esta ação".into());
    }

    match command {
        MediaCommand::Play => player.call_method("Play", &()).map(|_| ()),
        MediaCommand::Pause => player.call_method("Pause", &()).map(|_| ()),
        MediaCommand::Previous => player.call_method("Previous", &()).map(|_| ()),
        MediaCommand::Next => player.call_method("Next", &()).map(|_| ()),
        MediaCommand::Seek => {
            let offset_us = i64::from(offset.unwrap_or(0)) * 1_000_000;
            player.call_method("Seek", &(offset_us,)).map(|_| ())
        }
    }
    .map_err(|e| format!("Player recusou esta ação: {e}"))
}

impl MediaBackend for LinuxMedia {
    fn read(&self) -> MediaState {
        let (tx, rx) = mpsc::channel();
        if worker().send(Task::Read(tx)).is_err() {
            return MediaState {
                sessions: Vec::new(),
            };
        }
        rx.recv_timeout(Duration::from_secs(2))
            .unwrap_or(MediaState {
                sessions: Vec::new(),
            })
    }

    fn control(
        &self,
        id: &str,
        command: &MediaCommand,
        offset: Option<i8>,
        permit: Box<dyn FnOnce() -> bool + Send>,
    ) -> Result<(), PlatformError> {
        let (tx, rx) = mpsc::channel();
        worker()
            .send(Task::Control(
                id.to_string(),
                command.clone(),
                offset,
                permit,
                tx,
            ))
            .map_err(|_| PlatformError::Internal("Media worker stopped".to_string()))?;
        rx.recv()
            .map_err(|_| PlatformError::Internal("Media worker stopped".to_string()))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_helpers_extract_title_artist_and_duration() {
        let title_val = OwnedValue::try_from(Value::from("Bohemian Rhapsody")).unwrap();
        assert_eq!(
            extract_metadata_string(&title_val).as_deref(),
            Some("Bohemian Rhapsody")
        );
        let len_val = OwnedValue::try_from(Value::from(354_000_000i64)).unwrap();
        assert_eq!(extract_metadata_micros(&len_val), Some(354_000_000));
    }
}
