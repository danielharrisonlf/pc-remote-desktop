//! In-app updates through `tauri-plugin-updater`, driven by app commands.
//!
//! The webview has no updater permission of its own: it asks Rust to check, download and
//! install. The manifest (`latest.json` of the latest GitHub release) and the installer are
//! fetched here. Before the bytes are kept, the plugin verifies their minisign signature
//! against the public key in `tauri.conf.json`, including the version the signature was made
//! for (`requireSignedVersion`). The verified installer stays in memory until the user
//! restarts; install runs it in passive `/UPDATE` mode and exits so the files are unlocked,
//! and the installer relaunches the app when it finishes.

use crate::core::Core;
use semver::Version;
use serde::Serialize;
use std::{
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};
use tauri::{ipc::Channel, AppHandle, Manager, Runtime, State, Url};
use tauri_plugin_updater::{Error, Update, UpdaterExt};

const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// Progress is reported at most once per step, plus the last chunk.
const PROGRESS_STEP: u64 = 64 * 1024;
/// The manifest is not signed; its notes are only rendered as text, but stay bounded.
const MAX_NOTES_CHARS: usize = 20_000;
/// Debug builds only: replaces the manifest URL to test against a local server.
pub const ENDPOINT_OVERRIDE_VAR: &str = "PCREMOTE_UPDATE_ENDPOINT";

/// A newer version announced by the manifest.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub current_version: String,
    pub notes: String,
    /// Unix milliseconds, from the manifest `pub_date`.
    pub published_at: Option<i64>,
    /// The installer for this version is already downloaded and verified.
    pub ready: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum DownloadEvent {
    #[serde(rename_all = "camelCase")]
    Started { content_length: Option<u64> },
    #[serde(rename_all = "camelCase")]
    Progress {
        downloaded: u64,
        content_length: Option<u64>,
    },
    /// Every byte arrived; the signature is checked before the command resolves.
    Finished,
}

#[derive(Clone, Copy)]
enum Stage {
    Check,
    Download,
    Install,
}

#[derive(Default)]
pub struct UpdateState(Mutex<Slot>);

#[derive(Default)]
struct Slot {
    /// The update found by the latest successful check.
    pending: Option<Update>,
    /// Verified installer for `pending`.
    downloaded: Option<Downloaded>,
    downloading: bool,
}

struct Downloaded {
    version: String,
    bytes: Arc<Vec<u8>>,
}

/// Clears the in-progress flag however the download ends.
struct Downloading<'a>(&'a UpdateState);

impl Drop for Downloading<'_> {
    fn drop(&mut self) {
        self.0.lock().downloading = false;
    }
}

impl UpdateState {
    fn lock(&self) -> MutexGuard<'_, Slot> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Asks the manifest for a newer version. A failed check keeps what was found before.
    pub async fn check<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        endpoint: Option<Url>,
    ) -> Result<Option<UpdateInfo>, String> {
        let found: Result<Option<Update>, Error> = async {
            let mut builder = app.updater_builder().timeout(CHECK_TIMEOUT);
            if let Some(url) = endpoint {
                builder = builder.endpoints(vec![url])?;
            }
            builder.build()?.check().await
        }
        .await;
        let found = found.map_err(|error| failure(app, Stage::Check, &error))?;
        let mut slot = self.lock();
        let Some(update) = found else {
            slot.pending = None;
            slot.downloaded = None;
            return Ok(None);
        };
        if slot
            .downloaded
            .as_ref()
            .is_some_and(|downloaded| downloaded.version != update.version)
        {
            slot.downloaded = None;
        }
        let info = describe_update(&update, slot.downloaded.is_some());
        slot.pending = Some(update);
        Ok(Some(info))
    }

    /// Downloads the pending update and keeps it only after the signature verifies.
    pub async fn download<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        on_event: &(dyn Fn(DownloadEvent) + Sync),
    ) -> Result<(), String> {
        let mut update = {
            let mut slot = self.lock();
            let Some(update) = slot.pending.clone() else {
                return Err("Verifique se há atualizações antes de baixar.".into());
            };
            if slot
                .downloaded
                .as_ref()
                .is_some_and(|downloaded| downloaded.version == update.version)
            {
                on_event(DownloadEvent::Finished);
                return Ok(());
            }
            if slot.downloading {
                return Err("O download da atualização já está em andamento.".into());
            }
            slot.downloading = true;
            update
        };
        let _downloading = Downloading(self);
        update.timeout = Some(DOWNLOAD_TIMEOUT);
        let mut received = 0u64;
        let mut reported = 0u64;
        let mut started = false;
        let result = update
            .download(
                |chunk, content_length| {
                    if !started {
                        started = true;
                        on_event(DownloadEvent::Started { content_length });
                    }
                    received += chunk as u64;
                    if received - reported >= PROGRESS_STEP || content_length == Some(received) {
                        reported = received;
                        on_event(DownloadEvent::Progress {
                            downloaded: received,
                            content_length,
                        });
                    }
                },
                || on_event(DownloadEvent::Finished),
            )
            .await;
        let bytes = result.map_err(|error| failure(app, Stage::Download, &error))?;
        let mut slot = self.lock();
        if slot
            .pending
            .as_ref()
            .is_some_and(|pending| pending.version == update.version)
        {
            slot.downloaded = Some(Downloaded {
                version: update.version.clone(),
                bytes: Arc::new(bytes),
            });
            Ok(())
        } else {
            Err("Outra versão foi publicada durante o download. Tente de novo.".into())
        }
    }

    /// The verified installer, only for a version newer than the running one.
    pub fn prepared(&self, current: &Version) -> Result<(Update, Arc<Vec<u8>>), String> {
        let slot = self.lock();
        let (Some(update), Some(downloaded)) = (slot.pending.as_ref(), slot.downloaded.as_ref())
        else {
            return Err("Baixe a atualização antes de reiniciar.".into());
        };
        if downloaded.version != update.version || !is_newer(&update.version, current) {
            return Err("A atualização baixada não é mais nova que a versão em uso.".into());
        }
        Ok((update.clone(), downloaded.bytes.clone()))
    }
}

/// True only when `candidate` parses and has higher semver precedence than `current`.
pub fn is_newer(candidate: &str, current: &Version) -> bool {
    Version::parse(candidate.trim_start_matches('v')).is_ok_and(|version| version > *current)
}

/// Debug builds read [`ENDPOINT_OVERRIDE_VAR`] to test against a local server.
#[cfg(debug_assertions)]
pub fn endpoint_override() -> Option<Url> {
    let value = std::env::var(ENDPOINT_OVERRIDE_VAR).ok()?;
    let parsed = Url::parse(value.trim());
    if parsed.is_err() {
        eprintln!("{ENDPOINT_OVERRIDE_VAR} ignorado: URL inválida");
    }
    parsed.ok()
}

/// Release builds always use the endpoints in `tauri.conf.json`.
#[cfg(not(debug_assertions))]
pub fn endpoint_override() -> Option<Url> {
    None
}

fn describe_update(update: &Update, ready: bool) -> UpdateInfo {
    UpdateInfo {
        version: update.version.clone(),
        current_version: update.current_version.clone(),
        notes: update
            .body
            .as_deref()
            .unwrap_or_default()
            .replace("\r\n", "\n")
            .trim()
            .chars()
            .take(MAX_NOTES_CHARS)
            .collect(),
        published_at: update.date.map(|date| date.unix_timestamp() * 1000),
        ready,
    }
}

/// Records the technical error in Diagnóstico and returns plain pt-BR text for the panel.
fn failure<R: Runtime>(app: &AppHandle<R>, stage: Stage, error: &Error) -> String {
    if let Some(core) = app.try_state::<Arc<Core>>() {
        core.diagnostic(&format!("Atualização: {error}"));
    }
    message(stage, error)
}

fn message(stage: Stage, error: &Error) -> String {
    match error {
        Error::ReleaseNotFound => "Nenhuma atualização publicada foi encontrada.".into(),
        Error::Reqwest(error) if error.is_timeout() => {
            "O servidor de atualizações demorou demais para responder.".into()
        }
        Error::Reqwest(_) | Error::Network(_) => match stage {
            Stage::Download => {
                "O download foi interrompido. Verifique a conexão e tente de novo.".into()
            }
            _ => "Sem conexão com o servidor de atualizações.".into(),
        },
        Error::Serialization(_) | Error::Semver(_) | Error::UrlParse(_) => {
            "O arquivo de atualização publicado é inválido.".into()
        }
        Error::TargetNotFound(_) | Error::TargetsNotFound(_) => {
            "A versão publicada não tem instalador para este Windows.".into()
        }
        Error::Minisign(_)
        | Error::Base64(_)
        | Error::SignatureUtf8(_)
        | Error::SignedVersionMismatch { .. }
        | Error::MissingSignedVersion => {
            "A assinatura da atualização não confere. O arquivo baixado foi descartado.".into()
        }
        Error::InvalidUpdaterFormat => "O arquivo baixado não é um instalador do Windows.".into(),
        Error::InsecureTransportProtocol => "O endereço de atualização precisa usar HTTPS.".into(),
        Error::Io(_) if matches!(stage, Stage::Install) => {
            "Não foi possível iniciar o instalador.".into()
        }
        _ => format!("Não foi possível concluir a atualização ({error})."),
    }
}

#[tauri::command]
pub async fn update_check(
    app: AppHandle,
    state: State<'_, UpdateState>,
) -> Result<Option<UpdateInfo>, String> {
    state.check(&app, endpoint_override()).await
}

#[tauri::command]
pub async fn update_download(
    app: AppHandle,
    state: State<'_, UpdateState>,
    on_event: Channel<DownloadEvent>,
) -> Result<(), String> {
    let send = move |event: DownloadEvent| {
        let _ = on_event.send(event);
    };
    state.download(&app, &send).await
}

/// Starts the verified installer. On Windows the plugin runs `on_before_exit` (tray icon
/// cleanup), launches `setup.exe /P /UPDATE /R` and exits the process, which releases the
/// executable, the local server port and the single-instance lock before the files are
/// replaced. The installer relaunches the app.
#[tauri::command]
pub async fn update_install(app: AppHandle, state: State<'_, UpdateState>) -> Result<(), String> {
    let (update, bytes) = state.prepared(&app.package_info().version)?;
    let installed = tauri::async_runtime::spawn_blocking(move || update.install(bytes.as_slice()))
        .await
        .map_err(|_| "A instalação foi interrompida.".to_string())?;
    installed.map_err(|error| failure(&app, Stage::Install, &error))?;
    // Only reached if install() returns instead of exiting. On Windows the installer
    // relaunches the app; elsewhere the new version is already in place.
    if !crate::platform::system().exit_after_update_install() {
        app.exit(0);
        return Ok(());
    }
    app.restart()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn compares_versions_by_semver_precedence() {
        let installed = Version::new(0, 5, 0);
        assert!(is_newer("0.5.1", &installed));
        assert!(is_newer("v0.10.0", &installed));
        assert!(!is_newer("0.5.0", &installed));
        assert!(!is_newer("0.4.9", &installed));
        assert!(!is_newer("0.5.1-rc.1", &Version::new(0, 5, 1)));
        assert!(!is_newer("latest", &installed));
    }

    #[test]
    fn signature_failures_read_as_a_discarded_download() {
        let text = message(Stage::Download, &Error::MissingSignedVersion);
        assert_eq!(
            text,
            "A assinatura da atualização não confere. O arquivo baixado foi descartado."
        );
        assert_eq!(
            message(
                Stage::Download,
                &Error::SignedVersionMismatch {
                    signed: "0.5.1".into(),
                    announced: "0.6.0".into()
                }
            ),
            text
        );
        assert_eq!(
            message(Stage::Check, &Error::ReleaseNotFound),
            "Nenhuma atualização publicada foi encontrada."
        );
    }

    #[test]
    fn endpoint_override_is_read_only_in_debug_builds() {
        // Besides the ignored end-to-end test, only this test touches the variable.
        std::env::set_var(ENDPOINT_OVERRIDE_VAR, "http://127.0.0.1:9/latest.json");
        let overridden = endpoint_override();
        std::env::remove_var(ENDPOINT_OVERRIDE_VAR);
        if cfg!(debug_assertions) {
            assert_eq!(
                overridden.unwrap().as_str(),
                "http://127.0.0.1:9/latest.json"
            );
        } else {
            assert!(overridden.is_none());
        }
        assert!(endpoint_override().is_none());
    }

    #[test]
    fn download_events_use_camel_case_tags() {
        assert_eq!(
            serde_json::to_value(DownloadEvent::Progress {
                downloaded: 10,
                content_length: Some(20)
            })
            .unwrap(),
            json!({ "event": "progress", "data": { "downloaded": 10, "contentLength": 20 } })
        );
        assert_eq!(
            serde_json::to_value(DownloadEvent::Finished).unwrap(),
            json!({ "event": "finished" })
        );
    }
}
