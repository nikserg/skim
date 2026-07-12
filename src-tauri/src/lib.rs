pub mod commands;
pub mod db;
pub mod error;
pub mod mail;
pub mod secrets;
pub mod state;

use state::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "skim_lib=info".into()),
        )
        .init();

    // Several dependencies (reqwest, tokio-rustls) link rustls with different
    // crypto backends; pick one explicitly or every TLS handshake panics.
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // Focus the existing window instead of launching a second instance.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let db = db::Db::open(&data_dir.join("skim.db"))?;
            app.manage(AppState::new(db.clone()));

            // Resume syncing for known accounts.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = handle.state::<AppState>();
                let accounts = match state.db.call(|conn| db::accounts::list(conn)).await {
                    Ok(a) => a,
                    Err(e) => {
                        tracing::error!(error = %e, "cannot list accounts on startup");
                        return;
                    }
                };
                let mut engines = state.engines.lock().await;
                for account in accounts {
                    let sync_handle =
                        mail::sync::spawn(handle.clone(), state.db.clone(), account.clone());
                    engines.insert(account.id, sync_handle);
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::accounts::autoconfig_lookup,
            commands::accounts::google_oauth_available,
            commands::accounts::list_accounts,
            commands::accounts::add_account,
            commands::accounts::start_google_oauth,
            commands::accounts::remove_account,
            commands::mail::list_folders,
            commands::mail::list_threads,
            commands::mail::sync_now,
            commands::settings::get_settings,
            commands::settings::set_setting,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
