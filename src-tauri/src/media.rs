//! System media sessions and transport controls. Delegates to the active platform's `MediaBackend`.
use crate::protocol::{MediaCommand, MediaState};

pub fn read() -> MediaState {
    crate::platform::media().read()
}

pub fn control(
    id: &str,
    command: &MediaCommand,
    offset: Option<i8>,
    permit: impl FnOnce() -> bool + Send + 'static,
) -> Result<(), String> {
    crate::platform::media()
        .control(id, command, offset, Box::new(permit))
        .map_err(|e| e.into_legacy_message())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::friendly_app;

    #[test]
    fn friendly_names_for_common_players() {
        assert_eq!(friendly_app("Spotify.exe"), "Spotify");
        assert_eq!(friendly_app("Chrome"), "Google Chrome");
        assert_eq!(friendly_app("MSEdge"), "Microsoft Edge");
        assert_eq!(friendly_app("308046B0AF4A39CB"), "Firefox");
        assert_eq!(friendly_app("rhythmbox"), "Rhythmbox");
        assert_eq!(
            friendly_app("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"),
            "Reprodutor Multimídia"
        );
        assert_eq!(
            friendly_app("MSTeams_8wekyb3d8bbwe!MSTeams"),
            "Microsoft Teams"
        );
        assert_eq!(friendly_app("Contoso.Player"), "Contoso.Player");
    }

    #[test]
    fn queued_media_action_requires_current_permission_before_execution() {
        assert_eq!(
            control("old-session", &MediaCommand::Play, None, || false),
            Err("unauthorized".into())
        );
    }
}
