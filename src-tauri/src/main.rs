#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod logging;
mod state;

use std::time::Duration;

use tauri::Manager;

fn main() {
    let _log_guard = logging::init();
    // Tauri'nin varsayılanı çekirdek sayısı kadar işçi iş parçacığıdır; katalog sorguları G/Ç beklediği için iki yeter.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio çalışma zamanı kurulamadı");
    tauri::async_runtime::set(runtime.handle().clone());
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let state = state::AppState::init();
            let manager = state.manager.clone();
            tauri::async_runtime::spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_secs(60));
                loop {
                    tick.tick().await;
                    manager.reap_idle(Duration::from_secs(600)).await;
                }
            });
            app.manage(state);
            tracing::info!("pg-tools başlatıldı");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_servers,
            commands::save_server,
            commands::test_connection,
            commands::connect_server,
            commands::disconnect_server,
            commands::list_children,
            commands::get_properties,
            commands::get_ddl,
            commands::startup_notice,
            commands::delete_server,
            commands::set_show_system,
            commands::quote_identifiers,
            commands::preview_pgadmin_import,
            commands::apply_pgadmin_import,
        ])
        .run(tauri::generate_context!())
        .expect("pg-tools başlatılamadı");
}
