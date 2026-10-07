#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod dash;
mod data;
mod ddl;
mod logging;
mod query;
mod state;
mod store_cmds;
mod window;

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
        .on_page_load(|webview, payload| {
            // Sayfa yenilenince (F5, HMR, gezinme) arayüzün oturum kimlikleri kaybolur: yetim oturumları kapat.
            if payload.event() == tauri::webview::PageLoadEvent::Started
                && let Some(state) = webview.app_handle().try_state::<state::AppState>()
            {
                let query = state.query.clone();
                // Dashboard kimlikleri de arayüzle birlikte kaybolur: sahipsiz bağlantı kalmasın.
                state.dash.close_all();
                // Uçuştaki DDL uygulamasının sahibi (arayüz) gitti: iptal et (sunucu tarafı tx bağlantı düşünce geri alınır).
                state.ddl.cancel_all();
                state.data.cancel_all();
                tauri::async_runtime::spawn(async move { query::close_all_on_reload(&query).await });
            }
        })
        .setup(|app| {
            window::init(app);
            let state = state::AppState::init();
            let manager = state.manager.clone();
            let query = state.query.clone();
            tauri::async_runtime::spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_secs(60));
                loop {
                    tick.tick().await;
                    manager.reap_idle(Duration::from_secs(600)).await;
                    query.reap_idle(Duration::from_secs(30 * 60));
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
            window::app_ready,
            commands::delete_server,
            commands::set_show_system,
            commands::quote_identifiers,
            commands::qt_target_of,
            commands::preview_pgadmin_import,
            commands::apply_pgadmin_import,
            query::qt_open,
            query::qt_open_readonly,
            query::qt_analyze,
            query::qt_run,
            query::qt_more,
            query::qt_cancel,
            query::qt_tx,
            query::qt_tx_state,
            query::qt_set_options,
            query::qt_close,
            query::qt_complete,
            query::qt_check,
            query::qt_format,
            query::qt_export_csv,
            dash::dash_open,
            dash::dash_close,
            dash::dash_sample,
            dash::dash_overview,
            dash::dash_activity,
            dash::dash_locks,
            dash::dash_settings,
            dash::dash_prepared,
            dash::dash_statements,
            dash::dash_signal,
            ddl::ddl_target_of,
            ddl::ddl_defaults,
            ddl::ddl_load,
            ddl::ddl_lookup,
            ddl::ddl_preview,
            ddl::ddl_apply,
            ddl::ddl_cancel,
            data::data_load,
            data::data_select,
            data::data_preview,
            data::data_apply,
            data::data_cancel,
            store_cmds::qt_history_push_many,
            store_cmds::qt_history_list,
            store_cmds::qt_history_clear,
            store_cmds::qt_drafts_save,
            store_cmds::qt_drafts_remove,
            store_cmds::qt_drafts_load,
            store_cmds::qt_drafts_clear,
            store_cmds::settings_get,
            store_cmds::settings_set_history,
            store_cmds::settings_set_editor,
        ])
        .build(tauri::generate_context!())
        .expect("pg-tools başlatılamadı")
        .run(|app, event| {
            // Kapanışta açık sorgu oturumları kapatılır: açık transaction'lar ROLLBACK edilir (en iyi çaba, sınırlı bekleme).
            if let tauri::RunEvent::Exit = event {
                let state = app.state::<state::AppState>();
                let query = state.query.clone();
                state.dash.close_all();
                // Uçuştaki DDL iptalleri arka planda gönderilir; süreç bitmeden ulaşmaları için kısa bekleme.
                let ddl_cancels = state.ddl.cancel_all() + state.data.cancel_all();
                // Zaman aşımı geleceği çalışma zamanı İÇİNDE kurulmalı: dışarıda `tokio::time::timeout` "no reactor running" ile panikler.
                tauri::async_runtime::block_on(async move {
                    if ddl_cancels > 0 {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                    let _ = tokio::time::timeout(Duration::from_secs(7), query.close_all()).await;
                });
            }
        });
}
