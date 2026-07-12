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
        .register_uri_scheme_protocol("skim-cid", |ctx, request| {
            // Serves cached inline (cid:) images to the message iframe:
            // http://skim-cid.localhost/<message_pk>/<url-encoded content id>
            let not_found = || {
                tauri::http::Response::builder()
                    .status(404)
                    .body(Vec::new())
                    .expect("static response")
            };
            let path = request.uri().path().trim_start_matches('/').to_string();
            let Some((pk_str, cid_enc)) = path.split_once('/') else {
                return not_found();
            };
            let Ok(message_pk) = pk_str.parse::<i64>() else {
                return not_found();
            };
            let content_id = urlencoding_decode(cid_enc);
            let state = ctx.app_handle().state::<AppState>();
            let file = state
                .db
                .with(|conn| db::bodies::get_attachment_by_cid(conn, message_pk, &content_id))
                .ok()
                .flatten();
            let Some(file) = file else { return not_found() };
            let Some(path) = file.cache_path else {
                return not_found();
            };
            match std::fs::read(&path) {
                Ok(bytes) => tauri::http::Response::builder()
                    .status(200)
                    .header(
                        "content-type",
                        file.mime_type
                            .as_deref()
                            .unwrap_or("application/octet-stream"),
                    )
                    .body(bytes)
                    .unwrap_or_else(|_| not_found()),
                Err(_) => not_found(),
            }
        })
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let db = db::Db::open(&data_dir.join("skim.db"))?;
            app.manage(AppState::new(db.clone(), data_dir.clone()));

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
                    let sync_handle = mail::sync::spawn(
                        handle.clone(),
                        state.db.clone(),
                        account.clone(),
                        state.data_dir.clone(),
                    );
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
            commands::mail::get_thread,
            commands::mail::get_message_body,
            commands::mail::allow_remote_images,
            commands::mail::mark_read,
            commands::mail::set_starred,
            commands::mail::archive_messages,
            commands::mail::delete_messages,
            commands::mail::save_attachment,
            commands::mail::open_attachment,
            commands::mail::sync_now,
            commands::settings::get_settings,
            commands::settings::set_setting,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn urlencoding_decode(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (
                bytes.get(i + 1).and_then(|b| (*b as char).to_digit(16)),
                bytes.get(i + 2).and_then(|b| (*b as char).to_digit(16)),
            ) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
