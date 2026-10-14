use crate::{
    audio::with_com,
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

enum Task {
    Read(Sender<MediaState>),
    Control(
        String,
        MediaCommand,
        Option<i8>,
        Box<dyn FnOnce() -> bool + Send>,
        Sender<Result<(), String>>,
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
                    let mut registry = Vec::<Entry>::new();
                    while let Ok(task) = rx.recv() {
                        match task {
                            Task::Read(reply) => {
                                let _ = reply.send(read_inside(&mut registry));
                            }
                            Task::Control(id, command, offset, permit, reply) => {
                                if !permit() {
                                    let _ = reply.send(Err("unauthorized".into()));
                                    continue;
                                }
                                let _ = reply.send(control_inside(
                                    &mut registry,
                                    &id,
                                    &command,
                                    offset,
                                ));
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

fn refresh(registry: &mut Vec<Entry>) -> Result<(), String> {
    let manager = Manager::RequestAsync()
        .map_err(|e| e.to_string())?
        .join()
        .map_err(|e| e.to_string())?;
    let view = manager.GetSessions().map_err(|e| e.to_string())?;
    let mut current = Vec::new();
    for i in 0..view.Size().map_err(|e| e.to_string())? {
        let session = view.GetAt(i).map_err(|e| e.to_string())?;
        let id = registry
            .iter()
            .find(|e| e.session == session)
            .map(|e| e.id.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        current.push(Entry { id, session });
    }
    *registry = current;
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

fn read_inside(registry: &mut Vec<Entry>) -> MediaState {
    if refresh(registry).is_err() {
        registry.clear();
        return MediaState {
            sessions: Vec::new(),
        };
    }
    let sessions = registry
        .iter()
        .filter_map(|entry| {
            let session = &entry.session;
            let app = session.SourceAppUserModelId().ok()?.to_string();
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
    registry: &mut Vec<Entry>,
    id: &str,
    command: &MediaCommand,
    offset: Option<i8>,
) -> Result<(), String> {
    refresh(registry)?;
    let session = &registry
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

pub fn read() -> MediaState {
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
pub fn control(
    id: &str,
    command: &MediaCommand,
    offset: Option<i8>,
    permit: impl FnOnce() -> bool + Send + 'static,
) -> Result<(), String> {
    let (tx, rx) = mpsc::channel();
    worker()
        .send(Task::Control(
            id.to_string(),
            command.clone(),
            offset,
            Box::new(permit),
            tx,
        ))
        .map_err(|_| "Media worker stopped".to_string())?;
    rx.recv().map_err(|_| "Media worker stopped".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_gsmtc_read_smoke() {
        let _ = read();
    }
    #[test]
    fn queued_media_action_requires_current_permission_before_execution() {
        assert_eq!(
            control("old-session", &MediaCommand::Play, None, || false),
            Err("unauthorized".into())
        );
    }
}
