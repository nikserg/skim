pub mod ai;
pub mod commands;
pub mod db;
pub mod error;
pub mod mail;
pub mod notify;
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
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // A protocol-activated toast click relaunches Skim with a
            // `skim://…` argument, forwarded here to the running instance.
            if let Some(uri) = args.iter().find(|a| a.starts_with("skim://")) {
                notify::handle_skim_uri(app, uri, true);
            } else {
                // A plain second launch surfaces the running instance (it may
                // be hidden in the tray).
                show_main_window(app);
            }
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                // Visibility is controlled by us (tray / --minimized), not
                // by the previous session.
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::all()
                        & !tauri_plugin_window_state::StateFlags::VISIBLE,
                )
                .build(),
        )
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
        .on_window_event(|window, event| {
            // Closing the main window hides it to the tray; quitting is a
            // tray-menu action. Compose windows close normally.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir).ok();
            notify::register_aumid();
            notify::register_url_scheme();
            let db = db::Db::open(&data_dir.join("skim.db"))?;
            app.manage(AppState::new(db.clone(), data_dir.clone()));

            let locale = db
                .with(|conn| db::queries::get_setting(conn, "locale"))
                .ok()
                .flatten()
                .unwrap_or_else(|| "en".into());
            setup_tray(app.handle(), &locale)?;

            // Autostart is on by default. The registry entry is reconciled
            // with the stored preference on every launch — the uninstaller
            // removes the Run key, so a reinstall must recreate it.
            {
                use tauri_plugin_autostart::ManagerExt;
                let wanted = db
                    .with(|conn| db::queries::get_setting(conn, "autostart"))
                    .ok()
                    .flatten()
                    .is_none_or(|v| v == "1");
                let autolaunch = app.autolaunch();
                match (wanted, autolaunch.is_enabled().unwrap_or(false)) {
                    (true, false) => {
                        let _ = autolaunch.enable();
                    }
                    (false, true) => {
                        let _ = autolaunch.disable();
                    }
                    _ => {}
                }
            }

            // `--minimized` (autostart) keeps the window hidden in the tray.
            let minimized = std::env::args().any(|a| a == "--minimized");
            if !minimized {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }

            // Cold start from a toast click: stash the target so the frontend
            // opens it once its listeners attach (via take_pending_open). The
            // running-app case goes through the single-instance handler above.
            if let Some(uri) = std::env::args().find(|a| a.starts_with("skim://")) {
                notify::handle_skim_uri(app.handle(), &uri, false);
            }

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
            commands::mail::list_messages,
            commands::mail::get_thread,
            commands::mail::get_message_body,
            commands::mail::allow_remote_images,
            commands::mail::mark_read,
            commands::mail::set_starred,
            commands::mail::archive_messages,
            commands::mail::delete_messages,
            commands::mail::report_spam,
            commands::mail::unsubscribe,
            commands::mail::save_attachment,
            commands::mail::open_attachment,
            commands::mail::sync_now,
            commands::mail::take_pending_open,
            commands::compose::create_draft,
            commands::compose::get_draft,
            commands::compose::update_draft,
            commands::compose::delete_draft,
            commands::compose::get_reply_template,
            commands::compose::send_draft,
            commands::compose::open_compose_window,
            commands::compose::suggest_addresses,
            commands::compose::add_draft_attachment,
            commands::compose::list_draft_attachments,
            commands::compose::remove_draft_attachment,
            commands::invites::rsvp_invite,
            commands::ai::ai_set_key,
            commands::ai::ai_key_status,
            commands::ai::ai_clear_key,
            commands::ai::ai_cancel,
            commands::ai::ai_compose,
            commands::ai::ai_ask,
            commands::ai::ai_chat,
            commands::ai::ai_analyze_style,
            commands::ai::ai_recap,
            commands::search::search_messages,
            commands::search::thread_message_ids,
            commands::settings::get_settings,
            commands::settings::set_setting,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn tray_labels(locale: &str) -> (&'static str, &'static str) {
    match locale {
        "ru" => ("Открыть Skim", "Выход"),
        "de" => ("Skim öffnen", "Beenden"),
        "fr" => ("Ouvrir Skim", "Quitter"),
        "es" => ("Abrir Skim", "Salir"),
        "it" => ("Apri Skim", "Esci"),
        "pl" => ("Otwórz Skim", "Zakończ"),
        "sr" => ("Otvori Skim", "Izlaz"),
        "zh" => ("打开 Skim", "退出"),
        "ja" => ("Skim を開く", "終了"),
        "ko" => ("Skim 열기", "종료"),
        _ => ("Open Skim", "Quit"),
    }
}

fn setup_tray(app: &tauri::AppHandle, locale: &str) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let (open_label, quit_label) = tray_labels(locale);
    let open = MenuItem::with_id(app, "open", open_label, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", quit_label, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;

    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().expect("bundled icon").clone())
        .tooltip("Skim")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
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
