mod clipboard_sync;
mod commands;
mod dto;
mod media_bridge;
mod settings;
mod state;
mod transfers;
mod trash;

use tauri::Manager;

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let app_state = tauri::async_runtime::block_on(state::AppState::init(handle.clone()))
                .expect("Failed initializing HandShaker backend state");

            let initial_decorations =
                tauri::async_runtime::block_on(app_state.settings.read()).use_system_decorations;
            // tauri.conf.json already starts undecorated (`decorations: false`); only override if enabled
            // so GTK doesn't warn about calling gtk_window_set_titlebar() on a realized window.
            if initial_decorations {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.set_decorations(true);
                }
            }

            #[cfg(target_os = "linux")]
            if let Some(win) = app.get_webview_window("main") {
                use std::sync::atomic::{AtomicU64, Ordering};
                use std::time::{SystemTime, UNIX_EPOCH};
                use webkit2gtk::{WebProcessTerminationReason, WebViewExt};

                let last_reload_ms = std::sync::Arc::new(AtomicU64::new(0));
                let _ = win.with_webview(move |wv| {
                    let last_reload = std::sync::Arc::clone(&last_reload_ms);
                    wv.inner().connect_web_process_terminated(move |view, reason| {
                        eprintln!(
                            "[webkit] WebKitWebProcess terminated unexpectedly ({reason:?})"
                        );
                        if matches!(
                            reason,
                            WebProcessTerminationReason::Crashed
                                | WebProcessTerminationReason::ExceededMemoryLimit
                        ) {
                            let now_ms = SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .map(|d| d.as_millis() as u64)
                                .unwrap_or(0);
                            let prev = last_reload.swap(now_ms, Ordering::SeqCst);
                            if now_ms.saturating_sub(prev) >= 5_000 {
                                view.reload();
                            }
                        }
                    });
                });
            }

            app.manage(app_state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::update_settings,
            commands::window_minimize,
            commands::window_toggle_maximize,
            commands::window_close,
            commands::scan_devices,
            commands::list_trusted_devices,
            commands::remove_trusted_device,
            commands::get_connection_state,
            commands::connect_wifi,
            commands::connect_usb,
            commands::disconnect_session,
            commands::get_device_info,
            commands::list_dir,
            commands::create_folder,
            commands::rename_file,
            commands::delete_files,
            commands::list_trash_items,
            commands::restore_trash_items,
            commands::delete_trash_items,
            commands::empty_trash,
            commands::list_transfers,
            commands::enqueue_download,
            commands::enqueue_upload,
            commands::cancel_transfer,
            commands::clear_finished_transfers,
            commands::get_media_base_url,
            commands::get_media_playback_support,
            commands::open_media_externally,
            commands::get_photo_library,
            commands::get_video_library,
            commands::get_audio_library,
            commands::get_clipboard_history,
            commands::post_clipboard_text,
            commands::clear_clipboard_history,
            commands::delete_clipboard_item,
            commands::copy_to_linux_clipboard,
        ])
        .run(tauri::generate_context!())
        .expect("error while running HandShaker desktop application");
}
