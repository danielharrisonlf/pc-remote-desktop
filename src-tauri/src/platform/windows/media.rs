//! Windows GSMTC (`GlobalSystemMediaTransportControlsSessionManager`) implementation of `MediaBackend`.
use crate::{
    platform::{friendly_app, windows::audio::with_com, MediaBackend, PlatformError},
    protocol::{MediaCapabilities, MediaCommand, MediaSession, MediaState},
};
use std::sync::{
    mpsc::{self, Sender},
    OnceLock,
};
use uuid::Uuid;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
};

pub struct WindowsMedia;

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
    session: Session,
}

static WORKER: OnceLock<Sender<Task>> = OnceLock::new();

fn worker() -> &'static Sender<Task> {
    WORKER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Task>();
        std::thread::Builder::new()
            .name("pc-remote-media-mta".into())
            .spawn(move || {
                let _ = with_com(|| {
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
                    Ok(())
                });
            })
            .expect("Media worker thread");
        tx
    })
}

/// One manager for the worker's lifetime: sessions keep their COM identity, and with it
/// their UUID, across reads. A failed manager is dropped and requested again next time.
#[derive(Default)]
struct Registry {
    manager: Option<Manager>,
    entries: Vec<Entry>,
}

fn manager(registry: &mut Registry) -> Result<Manager, String> {
    if let Some(manager) = &registry.manager {
        return Ok(manager.clone());
    }
    let manager = Manager::RequestAsync()
        .map_err(|e| e.to_string())?
        .join()
        .map_err(|e| e.to_string())?;
    registry.manager = Some(manager.clone());
    Ok(manager)
}

/// Re-lists the sessions: Windows' current one first, then those playing, then the rest.
fn refresh(registry: &mut Registry) -> Result<(), String> {
    let manager = manager(registry)?;
    let listed = (|| {
        let view = manager.GetSessions()?;
        let mut sessions = Vec::new();
        for i in 0..view.Size()? {
            sessions.push(view.GetAt(i)?);
        }
        Ok::<_, windows::core::Error>(sessions)
    })();
    let sessions = match listed {
        Ok(sessions) => sessions,
        Err(error) => {
            registry.manager = None;
            return Err(error.to_string());
        }
    };
    let current = manager.GetCurrentSession().ok();
    let playing = |session: &Session| {
        session
            .GetPlaybackInfo()
            .and_then(|info| info.PlaybackStatus())
            .is_ok_and(|status| status == PlaybackStatus::Playing)
    };
    let mut entries: Vec<(u8, Entry)> = sessions
        .into_iter()
        .map(|session| {
            let id = registry
                .entries
                .iter()
                .find(|e| e.session == session)
                .map(|e| e.id.clone())
                .unwrap_or_else(|| Uuid::new_v4().to_string());
            let rank = if current.as_ref() == Some(&session) {
                0
            } else if playing(&session) {
                1
            } else {
                2
            };
            (rank, Entry { id, session })
        })
        .collect();
    // Stable: equal ranks keep the order Windows listed them in.
    entries.sort_by_key(|(rank, _)| *rank);
    registry.entries = entries.into_iter().map(|(_, entry)| entry).collect();
    Ok(())
}

fn timeline_bounds(session: &Session) -> Option<(i64, i64, i64)> {
    let t = session.GetTimelineProperties().ok()?;
    let min = t.MinSeekTime().ok()?.Duration;
    let max = t.MaxSeekTime().ok()?.Duration;
    let position = t.Position().ok()?.Duration;
    if max <= min {
        return None;
    }
    let info = session.GetPlaybackInfo().ok();
    let playing = info
        .as_ref()
        .and_then(|i| i.PlaybackStatus().ok())
        .is_some_and(|status| status == PlaybackStatus::Playing);
    let rate = info
        .as_ref()
        .and_then(|i| i.PlaybackRate().ok())
        .and_then(|r| r.Value().ok())
        .unwrap_or(1.0);
    let elapsed = if playing {
        t.LastUpdatedTime()
            .ok()
            .map(|updated| {
                let unix_ticks = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
                    / 100;
                let windows_ticks = unix_ticks.saturating_add(116_444_736_000_000_000u128);
                windows_ticks
                    .saturating_sub(updated.UniversalTime.max(0) as u128)
                    .min(i64::MAX as u128) as i64
            })
            .unwrap_or(0)
    } else {
        0
    };
    let predicted = position
        .saturating_add((elapsed as f64 * rate.clamp(0.0, 16.0)) as i64)
        .clamp(min, max);
    Some((min, max, predicted))
}

fn read_inside(registry: &mut Registry) -> MediaState {
    if refresh(registry).is_err() {
        registry.entries.clear();
        return MediaState {
            sessions: Vec::new(),
        };
    }
    let sessions = registry
        .entries
        .iter()
        .filter_map(|entry| {
            let session = &entry.session;
            let app = friendly_app(&session.SourceAppUserModelId().ok()?.to_string());
            let info = session.GetPlaybackInfo().ok()?;
            let controls = info.Controls().ok()?;
            let playing = info.PlaybackStatus().ok()? == PlaybackStatus::Playing;
            let bounds = timeline_bounds(session);
            let caps = MediaCapabilities {
                play: controls.IsPlayEnabled().unwrap_or(false),
                pause: controls.IsPauseEnabled().unwrap_or(false),
                previous: controls.IsPreviousEnabled().unwrap_or(false),
                next: controls.IsNextEnabled().unwrap_or(false),
                seek: controls.IsPlaybackPositionEnabled().unwrap_or(false) && bounds.is_some(),
            };
            let title = session
                .TryGetMediaPropertiesAsync()
                .ok()
                .and_then(|op| op.join().ok())
                .and_then(|p| p.Title().ok())
                .map(|s| s.to_string())
                .unwrap_or_default();
            Some(MediaSession {
                id: entry.id.clone(),
                app,
                title,
                playing,
                capabilities: caps,
                position_seconds: bounds.map(|(_, _, p)| p as f64 / 10_000_000.0),
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
    refresh(registry)?;
    let session = &registry
        .entries
        .iter()
        .find(|e| e.id == id)
        .ok_or("Sessão de mídia indisponível")?
        .session;
    let controls = session
        .GetPlaybackInfo()
        .and_then(|p| p.Controls())
        .map_err(|e| e.to_string())?;
    let allowed = match command {
        MediaCommand::Play => controls.IsPlayEnabled(),
        MediaCommand::Pause => controls.IsPauseEnabled(),
        MediaCommand::Previous => controls.IsPreviousEnabled(),
        MediaCommand::Next => controls.IsNextEnabled(),
        MediaCommand::Seek => controls.IsPlaybackPositionEnabled(),
    }
    .unwrap_or(false);
    if !allowed {
        return Err("Player não permite esta ação".into());
    }
    let result = match command {
        MediaCommand::Play => session.TryPlayAsync().map_err(|e| e.to_string())?.join(),
        MediaCommand::Pause => session.TryPauseAsync().map_err(|e| e.to_string())?.join(),
        MediaCommand::Previous => session
            .TrySkipPreviousAsync()
            .map_err(|e| e.to_string())?
            .join(),
        MediaCommand::Next => session
            .TrySkipNextAsync()
            .map_err(|e| e.to_string())?
            .join(),
        MediaCommand::Seek => {
            let (min, max, current) =
                timeline_bounds(session).ok_or("Player não oferece intervalo de busca")?;
            let next = current
                .saturating_add(offset.unwrap_or(0) as i64 * 10_000_000)
                .clamp(min, max);
            session
                .TryChangePlaybackPositionAsync(next)
                .map_err(|e| e.to_string())?
                .join()
        }
    }
    .map_err(|e| e.to_string())?;
    if result {
        Ok(())
    } else {
        Err("Player recusou esta ação".into())
    }
}

impl MediaBackend for WindowsMedia {
    fn read(&self) -> MediaState {
        let (tx, rx) = mpsc::channel();
        if worker().send(Task::Read(tx)).is_err() {
            return MediaState {
                sessions: Vec::new(),
            };
        }
        rx.recv().unwrap_or(MediaState {
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
    fn windows_gsmtc_read_smoke() {
        let _ = WindowsMedia.read();
    }

    /// Needs a player with an open session (YouTube in a browser, Spotify…).
    #[test]
    #[ignore = "requires an active media session on the host"]
    fn windows_gsmtc_session_ids_are_stable_across_reads() {
        let first = WindowsMedia.read();
        assert!(!first.sessions.is_empty(), "no media session");
        for _ in 0..5 {
            std::thread::sleep(std::time::Duration::from_millis(200));
            let again = WindowsMedia.read();
            let ids = |state: &MediaState| {
                let mut ids: Vec<_> = state.sessions.iter().map(|s| s.id.clone()).collect();
                ids.sort();
                ids
            };
            assert_eq!(ids(&first), ids(&again));
        }
    }
}
