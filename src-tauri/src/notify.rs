//! New-mail desktop notifications (Windows toasts) with a mark-read quick
//! action. Sent from the sync engine when new inbox mail arrives while the
//! window is unfocused; the "notifications" setting turns them off.

use crate::db::{queries, Db};
use crate::state::AppState;
use tauri::{AppHandle, Manager};
use tauri_winrt_notification::{IconCrop, Toast};

const AUMID: &str = "com.skim.app";

/// Where the toast icon lives. The notification platform silently drops
/// images under hidden directories — the whole AppData tree included — so
/// the icon goes to a visible per-user path (verified empirically on Win11).
/// Built with forward slashes: the toast XML embeds it into a file:/// URI.
pub fn toast_icon_path() -> Option<std::path::PathBuf> {
    let home = std::env::var("USERPROFILE").ok()?;
    Some(std::path::PathBuf::from(format!(
        "{}/.skim/notify-icon.png",
        home.replace('\\', "/")
    )))
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

    // Newest of the batch shapes the toast.
    let pks = message_pks.to_vec();
    let newest: Option<(Option<String>, Option<String>, Option<String>)> = db
        .call(move |conn| {
            use rusqlite::OptionalExtension;
            let placeholders = pks.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let params: Vec<&dyn rusqlite::ToSql> =
                pks.iter().map(|p| p as &dyn rusqlite::ToSql).collect();
            conn.query_row(
                &format!(
                    "SELECT from_name, from_addr, subject FROM messages
                     WHERE id IN ({placeholders}) ORDER BY date DESC LIMIT 1"
                ),
                params.as_slice(),
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
        })
        .await
        .ok()
        .flatten();
    let Some((from_name, from_addr, subject)) = newest else {
        return;
    };

    let title = from_name
        .filter(|s| !s.is_empty())
        .or(from_addr)
        .unwrap_or_else(|| "Skim".into());
    let mut body = subject.unwrap_or_default();
    if message_pks.len() > 1 {
        body.push_str(&format!("  (+{})", message_pks.len() - 1));
    }

    let ids_arg = format!(
        "read:{}",
        message_pks
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    let app_for_cb = app.clone();

    // Toast holds raw COM pointers (!Send) — build and show it entirely on
    // the blocking thread.
    let _ = tokio::task::spawn_blocking(move || {
        let mut toast = Toast::new(AUMID);
        // The brand logo on the toast itself, independent of the header
        // icon's cache.
        if let Some(icon) = toast_icon_path() {
            toast = toast.icon(&icon, IconCrop::Square, "Skim");
        }
        toast
            .title(&title)
            .text1(&body)
            .add_button(mark_read_label(&locale), &ids_arg)
            .on_activated(move |action| {
                match action.as_deref() {
                    Some(arg) if arg.starts_with("read:") => {
                        let ids: Vec<i64> = arg["read:".len()..]
                            .split(',')
                            .filter_map(|s| s.parse().ok())
                            .collect();
                        let app = app_for_cb.clone();
                        tauri::async_runtime::spawn(async move {
                            let state = app.state::<AppState>();
                            let _ =
                                crate::commands::mail::apply_read(&app, state.inner(), ids, true)
                                    .await;
                        });
                    }
                    _ => {
                        // Body click → bring the app forward.
                        if let Some(window) = app_for_cb.get_webview_window("main") {
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                    }
                }
                Ok(())
            })
            .show()
    })
    .await;
}
