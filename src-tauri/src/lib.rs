mod audio;
mod clipboard;
mod connection;
mod core;
mod dialog;
mod external;
mod files;
mod input;
mod media;
mod outbox;
mod pairing;
mod power;
mod protocol;
mod secrets;
mod settings;
mod system;
/// Public for the integration tests in tests/updater.rs.
pub mod update;

use crate::core::{Core, Pairing, Status};
use std::{sync::Arc, time::Duration};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    DragDropEvent, Emitter, Manager,
};
use tauri_plugin_notification::NotificationExt;

#[tauri::command]
fn get_status(core: tauri::State<'_, Arc<Core>>) -> Status {
    core.status()
}
#[tauri::command]
fn start_pairing(core: tauri::State<'_, Arc<Core>>) -> Pairing {
    core.start_pairing()
}
#[tauri::command]
fn approve_pairing(
    core: tauri::State<'_, Arc<Core>>,
    id: uuid::Uuid,
    approved: bool,
) -> Result<(), String> {
    core.approve_pairing(id, approved)
}
#[tauri::command]
fn revoke_device(core: tauri::State<'_, Arc<Core>>, id: uuid::Uuid) -> Result<(), String> {
    core.revoke(id)
}
#[tauri::command]
fn update_settings(
    core: tauri::State<'_, Arc<Core>>,
    settings: settings::Settings,
) -> Result<(), String> {
    core.update_settings(settings)
}
#[tauri::command]
fn stop_input(core: tauri::State<'_, Arc<Core>>) -> Result<(), String> {
    core.stop_input()
}
#[tauri::command]
fn unblock_input(core: tauri::State<'_, Arc<Core>>) -> Result<(), String> {
    core.unblock_input()
}
/// Opens the native picker and queues the chosen files; returns one message per file left out.
#[tauri::command]
async fn send_files_dialog(
    app: tauri::AppHandle,
    core: tauri::State<'_, Arc<Core>>,
) -> Result<Vec<String>, String> {
    let owner = app
        .get_webview_window("main")
        .and_then(|window| window.hwnd().ok())
        .map_or(0, |hwnd| hwnd.0 as isize);
    let paths = tauri::async_runtime::spawn_blocking(move || dialog::pick_files(owner))
        .await
        .map_err(|_| "Não foi possível abrir a escolha de arquivos.".to_string())??;
    Ok(core.enqueue_outgoing(paths))
}
#[tauri::command]
fn cancel_outgoing(core: tauri::State<'_, Arc<Core>>) {
    core.cancel_outgoing();
}
#[tauri::command]
fn clear_outgoing(core: tauri::State<'_, Arc<Core>>) {
    core.clear_outgoing();
}
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    external::open(&url)
}
#[tauri::command]
fn open_received_folder(core: tauri::State<'_, Arc<Core>>) -> Result<(), String> {
    let folder = core
        .received_folder()
        .map_err(|error| format!("Não foi possível abrir Downloads\\PC Remote: {error}"))?;
    external::open_folder(&folder)
}

pub fn run() {
    connection::install_crypto_provider();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_notification::init())
        // The webview gets no updater permission; the update_* commands drive it.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(update::UpdateState::default())
        .invoke_handler(tauri::generate_handler![
            get_status,
            start_pairing,
            approve_pairing,
            revoke_device,
            update_settings,
            stop_input,
            unblock_input,
            send_files_dialog,
            cancel_outgoing,
            clear_outgoing,
            open_url,
            open_received_folder,
            update::update_check,
            update::update_download,
            update::update_install
        ])
        .setup(|app| {
            let identity =
                secrets::load_or_create().expect("Não foi possível carregar a identidade DPAPI");
            let (tls, fingerprint) =
                connection::tls_config(&identity.certificate_pem, &identity.private_key_pem)
                    .expect("Certificado local inválido");
            let core = Core::new(identity, fingerprint);
            let handle = app.handle().clone();
            // Installed builds use the bundle identifier as AUMID; dev builds fall back.
            core.set_notifier(Box::new(move |title, body| {
                let _ = handle
                    .notification()
                    .builder()
                    .title(title)
                    .body(body)
                    .show();
            }));
            let server = core.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = connection::serve(server.clone(), tls).await {
                    server.set_service_running(false);
                    server.diagnostic(&format!("Servidor local: {e}"));
                }
            });
            let poll = core.clone();
            tauri::async_runtime::spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_millis(650));
                loop {
                    tick.tick().await;
                    poll.check_resume();
                    poll.outgoing_tick();
                    let core = poll.clone();
                    let _ = tokio::task::spawn_blocking(move || core.refresh()).await;
                }
            });
            let (tx, rx) = std::sync::mpsc::channel();
            clipboard::start_listener(tx);
            let listener = core.clone();
            std::thread::spawn(move || {
                while let Ok(seq) = rx.recv() {
                    listener.clipboard_event_local(seq)
                }
            });
            app.manage(core);
            let show = MenuItem::with_id(app, "show", "Abrir PC Remote", true, None::<&str>)?;
            let pause = MenuItem::with_id(
                app,
                "pause",
                "Ligar ou desligar sincronização em tempo real",
                true,
                None::<&str>,
            )?;
            let quit = MenuItem::with_id(app, "quit", "Encerrar", true, None::<&str>)?;
            let stop_input_item = MenuItem::with_id(
                app,
                "stop_input",
                "Interromper mouse e teclado",
                true,
                None::<&str>,
            )?;
            let menu = Menu::with_items(app, &[&show, &pause, &stop_input_item, &quit])?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "pause" => {
                        let state = app.state::<Arc<Core>>();
                        let mut settings = state.status().settings;
                        settings.realtime = !settings.realtime;
                        let _ = state.update_settings(settings);
                    }
                    "stop_input" => {
                        let _ = app.state::<Arc<Core>>().stop_input();
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            // Files dropped on the panel go to the phone; the paths never reach the webview code.
            tauri::WindowEvent::DragDrop(DragDropEvent::Drop { paths, .. })
                if !paths.is_empty() =>
            {
                let problems = window.state::<Arc<Core>>().enqueue_outgoing(paths.clone());
                let _ = window.emit("outgoing-problems", problems);
            }
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("Falha ao iniciar PC Remote");
}
