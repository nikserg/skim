//! New-mail desktop notifications (Windows toasts) with a mark-read quick
//! action; a body click focuses the window and opens the newest message's
//! thread. Sent from the sync engine when new inbox mail arrives while the
//! window is unfocused; the "notifications" setting turns them off.

use crate::db::{queries, Db};
use crate::state::AppState;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};
use windows::core::HSTRING;
use windows::Data::Xml::Dom::XmlDocument;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};

const AUMID: &str = "com.skim.app";
const URL_SCHEME: &str = "skim";

/// Where the toast icon lives. The notification platform silently drops
/// images under hidden directories — the whole AppData tree included — so
/// the icon goes to a visible per-user path (verified empirically on Win11).
pub fn toast_icon_path() -> Option<std::path::PathBuf> {
    let home = std::env::var("USERPROFILE").ok()?;
    Some(
        std::path::Path::new(&home)
            .join(".skim")
            .join("notify-icon.png"),
    )
}

/// Register the AppUserModelID so toasts carry Skim's name and icon.
///
/// Note: the registry entry alone is not enough on current Windows 11
/// builds — toasts actually display because the installer's Start Menu
/// shortcut carries `System.AppUserModel.ID`. This entry supplements it
/// with the display name and icon.
pub fn register_aumid() {
    use winreg::enums::{HKEY_CURRENT_USER, REG_EXPAND_SZ};
    use winreg::{RegKey, RegValue};

    fn expand_sz(value: &str) -> RegValue {
        let bytes: Vec<u8> = value
            .encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(|u| u.to_le_bytes())
            .collect();
        RegValue {
            vtype: REG_EXPAND_SZ,
            bytes,
        }
    }

    let Some(icon_path) = toast_icon_path() else {
        return;
    };
    if let Some(dir) = icon_path.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let _ = std::fs::write(&icon_path, include_bytes!("../icons/128x128.png"));

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok((key, _)) = hkcu.create_subkey(format!(r"Software\Classes\AppUserModelId\{AUMID}")) {
        let _ = key.set_raw_value("DisplayName", &expand_sz("Skim"));
        let _ = key.set_raw_value("IconUri", &expand_sz(&icon_path.to_string_lossy()));
    }
}

/// Register the `skim:` URL scheme so protocol-activated toasts (and any
/// `skim://…` link) launch the app with the URI as its argument. Toast body
/// and button clicks use `activationType="protocol"`, which the shell routes
/// here — no COM activator or in-process message pump required (the reason
/// the previous in-process `on_activated` callback never fired).
pub fn register_url_scheme() {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let exe = exe.to_string_lossy().replace('/', "\\");

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok((key, _)) = hkcu.create_subkey(format!(r"Software\Classes\{URL_SCHEME}")) {
        let _ = key.set_value("", &"URL:Skim Protocol");
        let _ = key.set_value("URL Protocol", &"");
    }
    if let Ok((key, _)) =
        hkcu.create_subkey(format!(r"Software\Classes\{URL_SCHEME}\shell\open\command"))
    {
        let _ = key.set_value("", &format!("\"{exe}\" \"%1\""));
    }
}

fn mark_read_label(locale: &str) -> &'static str {
    match locale {
        "ru" => "Прочитано",
        "de" => "Gelesen",
        "fr" => "Lu",
        "es" => "Leído",
        "it" => "Letto",
        "pl" => "Przeczytane",
        "sr" => "Pročitano",
        "zh" => "标为已读",
        "ja" => "既読にする",
        "ko" => "읽음",
        _ => "Mark read",
    }
}

/// Show a toast for freshly arrived inbox messages.
pub async fn notify_new_mail(app: &AppHandle, db: &Db, message_pks: &[i64]) {
    if message_pks.is_empty() {
        return;
    }

    // Respect the user's setting (default: on).
    let (enabled, locale) = db
        .call(|conn| {
            Ok((
                queries::get_setting(conn, "notifications")?
                    .map(|v| v != "off")
                    .unwrap_or(true),
                queries::get_setting(conn, "locale")?.unwrap_or_else(|| "en".into()),
            ))
        })
        .await
        .unwrap_or((true, "en".into()));
    if !enabled {
        return;
    }

    // Only when the app is in the background.
    if let Some(window) = app.get_webview_window("main") {
        if window.is_focused().unwrap_or(false) {
            return;
        }
    }

    // Newest of the batch shapes the toast (and is what a body click opens).
    let pks = message_pks.to_vec();
    type NewestRow = (
        Option<String>,
        Option<String>,
        Option<String>,
        i64,
        Option<i64>,
    );
    let newest: Option<NewestRow> = db
        .call(move |conn| {
            use rusqlite::OptionalExtension;
            let placeholders = pks.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let params: Vec<&dyn rusqlite::ToSql> =
                pks.iter().map(|p| p as &dyn rusqlite::ToSql).collect();
            conn.query_row(
                &format!(
                    "SELECT from_name, from_addr, subject, folder_id, thread_id FROM messages
                     WHERE id IN ({placeholders}) ORDER BY date DESC LIMIT 1"
                ),
                params.as_slice(),
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()
        })
        .await
        .ok()
        .flatten();
    let Some((from_name, from_addr, subject, folder_id, thread_id)) = newest else {
        return;
    };
    let open_target = thread_id.map(|tid| (folder_id, tid));

    let title = from_name
        .filter(|s| !s.is_empty())
        .or(from_addr)
        .unwrap_or_else(|| "Skim".into());
    let mut body = subject.unwrap_or_default();
    if message_pks.len() > 1 {
        body.push_str(&format!("  (+{})", message_pks.len() - 1));
    }

    let ids_csv = message_pks
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let open_uri = match open_target {
        Some((folder_id, thread_id)) => {
            format!("skim://open?folder={folder_id}&thread={thread_id}")
        }
        None => "skim://open".to_string(),
    };
    let read_uri = format!("skim://read?ids={ids_csv}");
    let mark_read = mark_read_label(&locale);

    // Protocol-activated toast. A body click launches `open_uri`, the button
    // `read_uri`; Windows shell-launches these through the registered `skim:`
    // scheme (register_url_scheme), so no COM activator or in-process message
    // pump is needed — which is why the old in-process callback never fired.
    // The header logo comes from the AUMID (Start Menu shortcut + IconUri),
    // so no image goes inside the toast body.
    let xml = format!(
        r#"<toast activationType="protocol" launch="{launch}">
    <visual>
        <binding template="ToastGeneric">
            <text>{title}</text>
            <text>{body}</text>
        </binding>
    </visual>
    <actions>
        <action content="{mark}" activationType="protocol" arguments="{read}"/>
    </actions>
</toast>"#,
        launch = xml_escape(&open_uri),
        title = xml_escape(&title),
        body = xml_escape(&body),
        mark = xml_escape(mark_read),
        read = xml_escape(&read_uri),
    );

    // WinRT toast objects are !Send — build and show on a blocking thread.
    let _ = tokio::task::spawn_blocking(move || {
        if let Err(e) = show_toast_xml(&xml) {
            tracing::warn!(error = %e, "failed to show toast");
        }
    })
    .await;
}

/// Build and display a toast from a raw XML document under Skim's AUMID.
fn show_toast_xml(xml: &str) -> windows::core::Result<()> {
    let doc = XmlDocument::new()?;
    doc.LoadXml(&HSTRING::from(xml))?;
    let toast = ToastNotification::CreateToastNotification(&doc)?;
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))?;
    notifier.Show(&toast)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Route a `skim://…` activation URI into the app. Called from the
/// single-instance handler (running app: `frontend_ready = true`) and from
/// cold-start argv parsing (`frontend_ready = false`, no listener attached
/// yet — the target is stashed for `take_pending_open`).
pub fn handle_skim_uri(app: &AppHandle, uri: &str, frontend_ready: bool) {
    let Some(rest) = uri.strip_prefix("skim://") else {
        return;
    };
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    match path.trim_end_matches('/') {
        "open" => {
            if frontend_ready {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
            let folder = query_value(query, "folder").and_then(|v| v.parse::<i64>().ok());
            let thread = query_value(query, "thread").and_then(|v| v.parse::<i64>().ok());
            if let (Some(folder_id), Some(thread_id)) = (folder, thread) {
                if frontend_ready {
                    let _ = app.emit(
                        "mail:open-thread",
                        json!({ "folderId": folder_id, "threadId": thread_id }),
                    );
                } else if let Some(state) = app.try_state::<AppState>() {
                    *state.pending_open.lock().unwrap() = Some((folder_id, thread_id));
                }
            }
        }
        "read" => {
            let ids: Vec<i64> = query_value(query, "ids")
                .map(|v| v.split(',').filter_map(|s| s.parse().ok()).collect())
                .unwrap_or_default();
            if !ids.is_empty() {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let state = app.state::<AppState>();
                    let _ = crate::commands::mail::apply_read(&app, state.inner(), ids, true).await;
                });
            }
        }
        _ => {}
    }
}

fn query_value<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then_some(v)
    })
}
