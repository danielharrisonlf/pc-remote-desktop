mod audio;
mod clipboard;
mod connection;
mod core;
mod media;
mod pairing;
mod protocol;
mod secrets;
mod settings;

use crate::core::{Core, Pairing, Status};
use std::{sync::Arc, time::Duration};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager,
};

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

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .invoke_handler(tauri::generate_handler![
            get_status,
            start_pairing,
            approve_pairing,
            revoke_device,
            update_settings
        ])
        .setup(|app| {
            let identity =
                secrets::load_or_create().expect("Não foi possível carregar a identidade DPAPI");
            let (tls, fingerprint) =
                connection::tls_config(&identity.certificate_pem, &identity.private_key_pem)
                    .expect("Certificado local inválido");
            let core = Core::new(identity, fingerprint);
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
                "Pausar ou retomar sincronização",
                true,
                None::<&str>,
            )?;
            let quit = MenuItem::with_id(app, "quit", "Encerrar", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &pause, &quit])?;
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
                        settings.paused = !settings.paused;
                        let _ = state.update_settings(settings);
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("Falha ao iniciar PC Remote");
}
