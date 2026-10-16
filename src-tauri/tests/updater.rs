//! Updater flow against a local HTTP server, through a MockRuntime app configured like
//! `tauri.conf.json`. It lives here, not in `src/update.rs`, because the mock app links
//! comctl32 v6 imports: build.rs embeds the Common Controls manifest only into test
//! targets (`rustc-link-arg-tests`), so the release executable keeps the manifest that
//! tauri-build generates.

use pc_remote_desktop_lib::update::{
    endpoint_override, DownloadEvent, UpdateInfo, UpdateState, ENDPOINT_OVERRIDE_VAR,
};
use semver::Version;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};
use tauri::Url;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// Throwaway keypair made with `tauri signer generate` only for these fixtures; its private
// key was discarded. FIXTURE_SIGNATURE was made with `tauri signer sign --app-version 0.5.1`
// and UNVERSIONED_SIGNATURE without `--app-version`, both over FIXTURE.
const FIXTURE: &[u8] = b"PC Remote updater test fixture: not an installer.\n";
const FIXTURE_PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEYzNDFBMzg5OTJEN0M3NEMKUldSTXg5ZVNpYU5CODdidFpVbkZSdjV0bGRWVVUvaDVtZEs1aUtnRTJlK25vN0IvclhUMnp2TDcK";
const FIXTURE_SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVSTXg5ZVNpYU5COHdaeXA3a2pQbC9weWhmSGlkcERDV0c0eGJ2TmNLRGNPOVFuTS80T1NJQkp0YnptdzY3ajZYQ3RTNlY3aDFrTXg5MkFFMVN4T0xxdCtPYmdQV2lHWGdFPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkyMTU4NzA5CWZpbGU6Zml4dHVyZS5iaW4JdmVyc2lvbjowLjUuMQpQV1A0ODZiMlhzZzBOR2dtb0w1b0ExcDJIQTBYSUxzU0Y2aXNzMDR1VEJCUGJDSlRGczhtc1MwekQwYWRBUTJIOGljT2JNQUdmai90UWdwYTVlRmFEQT09Cg==";
const UNVERSIONED_SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVSTXg5ZVNpYU5CODVkTlkyQlVhZ1BrK2FZRmpjRUN6QUMvWG1sQ2lBc2JXaWtpSXk3MFlxS2pSS242aGo0WHRIZndWb1R0UHh4cS8zVnk2NmNKcFhSb3lLZ2dKeWh0T0FFPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkyMTU4NzEwCWZpbGU6Zml4dHVyZS1ub3Zlci5iaW4KWEJsRWNHOGdZUFFxVlFZSXhZRUJNOERZeVZRZHlnTE8rVzA1anJJc0RURmQ0ZE0yMHRDZ3E3YWtpK3NLM1RxUkVUM0FUWEpEQU1pdXc0S3dsT3c4RFE9PQo=";

struct Route {
    path: String,
    status: u16,
    body: Vec<u8>,
}

/// Minimal HTTP/1.1 server on 127.0.0.1 answering from a fixed table, which is built
/// from the server's base URL so manifests can point back at it.
async fn serve(routes: impl FnOnce(&str) -> Vec<Route>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let routes = Arc::new(routes(&base));
    tauri::async_runtime::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let routes = routes.clone();
            tauri::async_runtime::spawn(async move {
                let mut head = Vec::new();
                let mut buffer = [0u8; 4096];
                while !head.windows(4).any(|window| window == b"\r\n\r\n") {
                    match socket.read(&mut buffer).await {
                        Ok(0) | Err(_) => return,
                        Ok(read) => head.extend_from_slice(&buffer[..read]),
                    }
                }
                let request = String::from_utf8_lossy(&head);
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                let (status, body) = routes
                    .iter()
                    .find(|route| route.path == path)
                    .map(|route| (route.status, route.body.clone()))
                    .unwrap_or((404, b"not found".to_vec()));
                let reason = if status == 200 { "OK" } else { "Not Found" };
                let header = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(header.as_bytes()).await;
                let _ = socket.write_all(&body).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    base
}

fn manifest(version: &str, url: &str, signature: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "version": version,
        "notes": "- Correções\r\n- Atualização no app",
        "pub_date": "2026-10-16T12:00:00Z",
        "platforms": {
            "windows-x86_64-nsis": { "signature": signature, "url": url },
            "windows-x86_64": { "signature": signature, "url": url }
        }
    }))
    .unwrap()
}

/// A MockRuntime app running `version` with the given updater plugin configuration.
fn app_with(updater: Value, version: Version) -> tauri::App<MockRuntime> {
    let mut context = mock_context(noop_assets());
    context.package_info_mut().version = version;
    context
        .config_mut()
        .plugins
        .0
        .insert("updater".into(), updater);
    mock_builder()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .build(context)
        .unwrap()
}

/// Version 0.5.0, configured like tauri.conf.json except for the public key.
fn fixture_app() -> tauri::App<MockRuntime> {
    app_with(
        json!({
            "pubkey": FIXTURE_PUBKEY,
            "endpoints": ["https://example.invalid/latest.json"],
            "requireSignedVersion": true,
            "windows": { "installMode": "passive" }
        }),
        current(),
    )
}

struct Scenario {
    version: &'static str,
    signature: &'static str,
    served: Vec<u8>,
}

impl Scenario {
    fn valid() -> Self {
        Self {
            version: "0.5.1",
            signature: FIXTURE_SIGNATURE,
            served: FIXTURE.to_vec(),
        }
    }

    async fn start(self) -> Url {
        let base = serve(|base| {
            vec![
                Route {
                    path: "/latest.json".into(),
                    status: 200,
                    body: manifest(self.version, &format!("{base}/setup.exe"), self.signature),
                },
                Route {
                    path: "/setup.exe".into(),
                    status: 200,
                    body: self.served,
                },
            ]
        })
        .await;
        Url::parse(&format!("{base}/latest.json")).unwrap()
    }
}

fn collect() -> (
    Arc<Mutex<Vec<DownloadEvent>>>,
    impl Fn(DownloadEvent) + Sync,
) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    (events, move |event| sink.lock().unwrap().push(event))
}

fn assert_signature_rejected(result: Result<(), String>) {
    let error = result.unwrap_err();
    assert_eq!(
        error,
        "A assinatura da atualização não confere. O arquivo baixado foi descartado."
    );
}

fn current() -> Version {
    Version::new(0, 5, 0)
}

#[test]
fn finds_downloads_and_verifies_a_newer_signed_release() {
    let app = fixture_app();
    let handle = app.handle().clone();
    tauri::async_runtime::block_on(async move {
        let endpoint = Scenario::valid().start().await;
        let state = UpdateState::default();
        let info = state.check(&handle, Some(endpoint)).await.unwrap().unwrap();
        assert_eq!(
            info,
            UpdateInfo {
                version: "0.5.1".into(),
                current_version: "0.5.0".into(),
                notes: "- Correções\n- Atualização no app".into(),
                published_at: Some(1_792_152_000_000),
                ready: false,
            }
        );
        assert!(
            state.prepared(&current()).is_err(),
            "nothing downloaded yet"
        );

        let (events, sink) = collect();
        state.download(&handle, &sink).await.unwrap();
        let events = events.lock().unwrap().clone();
        let length = Some(FIXTURE.len() as u64);
        assert_eq!(
            events,
            vec![
                DownloadEvent::Started {
                    content_length: length
                },
                DownloadEvent::Progress {
                    downloaded: FIXTURE.len() as u64,
                    content_length: length
                },
                DownloadEvent::Finished,
            ]
        );
        let (update, bytes) = state.prepared(&current()).unwrap();
        assert_eq!(update.version, "0.5.1");
        assert_eq!(bytes.as_slice(), FIXTURE);
        // Already running 0.5.1: the downloaded bytes must not be installed.
        assert!(state.prepared(&Version::new(0, 5, 1)).is_err());
    });
}

#[test]
fn rejects_tampered_bytes_and_keeps_nothing() {
    let app = fixture_app();
    let handle = app.handle().clone();
    tauri::async_runtime::block_on(async move {
        let mut tampered = FIXTURE.to_vec();
        tampered[0] ^= 0x01;
        let endpoint = Scenario {
            served: tampered,
            ..Scenario::valid()
        }
        .start()
        .await;
        let state = UpdateState::default();
        assert!(state
            .check(&handle, Some(endpoint))
            .await
            .unwrap()
            .is_some());
        let (_, sink) = collect();
        let error = state.download(&handle, &sink).await.unwrap_err();
        assert!(error.starts_with("A assinatura da atualização não confere"));
        assert!(state.prepared(&current()).is_err());
    });
}

#[test]
fn rejects_a_manifest_that_relabels_a_signed_release() {
    let app = fixture_app();
    let handle = app.handle().clone();
    tauri::async_runtime::block_on(async move {
        // Signed for 0.5.1, announced as 0.6.0.
        let relabeled = Scenario {
            version: "0.6.0",
            ..Scenario::valid()
        }
        .start()
        .await;
        let state = UpdateState::default();
        state
            .check(&handle, Some(relabeled))
            .await
            .unwrap()
            .unwrap();
        let (_, sink) = collect();
        assert_signature_rejected(state.download(&handle, &sink).await);

        // A signature without a version is refused while requireSignedVersion is on.
        let unversioned = Scenario {
            signature: UNVERSIONED_SIGNATURE,
            ..Scenario::valid()
        }
        .start()
        .await;
        state
            .check(&handle, Some(unversioned))
            .await
            .unwrap()
            .unwrap();
        assert_signature_rejected(state.download(&handle, &sink).await);

        let garbage = Scenario {
            signature: "bm90IGEgc2lnbmF0dXJl",
            ..Scenario::valid()
        }
        .start()
        .await;
        state.check(&handle, Some(garbage)).await.unwrap().unwrap();
        assert_signature_rejected(state.download(&handle, &sink).await);
        assert!(state.prepared(&current()).is_err());
    });
}

#[test]
fn same_or_older_versions_and_missing_feeds_are_not_updates() {
    let app = fixture_app();
    let handle = app.handle().clone();
    tauri::async_runtime::block_on(async move {
        let state = UpdateState::default();
        for version in ["0.5.0", "0.4.0"] {
            let endpoint = Scenario {
                version,
                ..Scenario::valid()
            }
            .start()
            .await;
            assert_eq!(state.check(&handle, Some(endpoint)).await.unwrap(), None);
        }
        let missing = serve(|_| Vec::new()).await;
        let error = state
            .check(
                &handle,
                Some(Url::parse(&format!("{missing}/latest.json")).unwrap()),
            )
            .await
            .unwrap_err();
        assert_eq!(error, "Nenhuma atualização publicada foi encontrada.");
        let (_, sink) = collect();
        assert_eq!(
            state.download(&handle, &sink).await.unwrap_err(),
            "Verifique se há atualizações antes de baixar."
        );
    });
}

#[test]
fn a_new_check_keeps_the_download_only_for_the_same_version() {
    let app = fixture_app();
    let handle = app.handle().clone();
    tauri::async_runtime::block_on(async move {
        let state = UpdateState::default();
        let endpoint = Scenario::valid().start().await;
        state.check(&handle, Some(endpoint.clone())).await.unwrap();
        let (_, sink) = collect();
        state.download(&handle, &sink).await.unwrap();
        let again = state.check(&handle, Some(endpoint)).await.unwrap().unwrap();
        assert!(again.ready);

        let newer = Scenario {
            version: "0.5.2",
            ..Scenario::valid()
        }
        .start()
        .await;
        let info = state.check(&handle, Some(newer)).await.unwrap().unwrap();
        assert_eq!(info.version, "0.5.2");
        assert!(!info.ready);
        assert!(state.prepared(&current()).is_err());
    });
}

/// Real artifacts from `npm run tauri build` (signed with the release key) plus the
/// manifest from `npm run release:updater`, served locally. Run with:
/// `PCREMOTE_UPDATER_E2E_DIR=<dir with latest.json, the setup.exe and its .sig>
/// cargo test --target x86_64-pc-windows-msvc real_release_artifacts -- --ignored`.
/// It checks and downloads through PCREMOTE_UPDATE_ENDPOINT as an older install (0.0.0)
/// with the real tauri.conf.json updater settings, but never installs.
#[test]
#[ignore = "needs signed release artifacts in PCREMOTE_UPDATER_E2E_DIR"]
fn real_release_artifacts_verify_with_the_configured_key() {
    let dir = std::path::PathBuf::from(
        std::env::var("PCREMOTE_UPDATER_E2E_DIR").expect("PCREMOTE_UPDATER_E2E_DIR"),
    );
    let config: Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
    let older = Version::new(0, 0, 0);
    let app = app_with(config["plugins"]["updater"].clone(), older.clone());
    let handle = app.handle().clone();
    let published: Value =
        serde_json::from_slice(&std::fs::read(dir.join("latest.json")).unwrap()).unwrap();
    let version = published["version"].as_str().unwrap().to_string();
    let platform = &published["platforms"]["windows-x86_64"];
    let signature = platform["signature"].as_str().unwrap().to_string();
    let name = platform["url"]
        .as_str()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap()
        .to_string();
    let installer = std::fs::read(dir.join(&name)).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join(format!("{name}.sig")))
            .unwrap()
            .trim(),
        signature,
        "latest.json must carry the .sig of the uploaded installer"
    );
    tauri::async_runtime::block_on(async move {
        let mut tampered = installer.clone();
        let middle = tampered.len() / 2;
        tampered[middle] ^= 0x01;
        // Same manifest, with every platform URL pointed at the local server.
        let relocated = |url: String| {
            let mut manifest = published.clone();
            if let Some(platforms) = manifest["platforms"].as_object_mut() {
                for entry in platforms.values_mut() {
                    entry["url"] = json!(url);
                }
            }
            serde_json::to_vec(&manifest).unwrap()
        };
        let feed = serve(|base| {
            vec![
                Route {
                    path: "/latest.json".into(),
                    status: 200,
                    body: relocated(format!("{base}/{name}")),
                },
                Route {
                    path: "/bad.json".into(),
                    status: 200,
                    body: relocated(format!("{base}/tampered.exe")),
                },
                Route {
                    path: format!("/{name}"),
                    status: 200,
                    body: installer.clone(),
                },
                Route {
                    path: "/tampered.exe".into(),
                    status: 200,
                    body: tampered,
                },
            ]
        })
        .await;

        std::env::set_var(ENDPOINT_OVERRIDE_VAR, format!("{feed}/latest.json"));
        let endpoint = endpoint_override();
        std::env::remove_var(ENDPOINT_OVERRIDE_VAR);
        let state = UpdateState::default();
        let info = state.check(&handle, endpoint).await.unwrap().unwrap();
        assert_eq!(info.version, version);
        let (events, sink) = collect();
        state.download(&handle, &sink).await.unwrap();
        assert!(matches!(
            events.lock().unwrap().last(),
            Some(DownloadEvent::Finished)
        ));
        let (update, bytes) = state.prepared(&older).unwrap();
        assert_eq!(update.version, version);
        assert_eq!(bytes.as_slice(), installer.as_slice());
        println!(
            "verified {name}: {} bytes, version {version}",
            installer.len()
        );

        let rejected = UpdateState::default();
        rejected
            .check(
                &handle,
                Some(Url::parse(&format!("{feed}/bad.json")).unwrap()),
            )
            .await
            .unwrap()
            .unwrap();
        let error = rejected.download(&handle, &sink).await.unwrap_err();
        assert!(error.starts_with("A assinatura da atualização não confere"));
        println!("tampered copy rejected: {error}");
    });
}
