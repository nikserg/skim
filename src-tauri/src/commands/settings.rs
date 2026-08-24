use crate::db::queries;
use crate::error::{Result, SkimError};
use crate::state::AppState;
use std::collections::HashMap;
use tauri::State;

/// Keys the frontend may read/write. Everything else lives in Rust only.
const ALLOWED: &[&str] = &[
    "active_account",
    "locale",
    "theme",
    "images_policy",
    "notifications",
    "group_threads",
    "last_from_account",
    "sidebar_collapsed",
    "autostart",
    "ai_model",
    "ai_provider",
    "openrouter_model",
    "custom_base_url",
    "custom_model",
    "ai_user_name",
    "ai_style",
    "ai_style_profile",
    "ai_instructions",
    "update_last_check",
    "update_dismissed",
];

/// A package manager (Scoop today) that owns the install drops this file next
/// to the exe. Our NSIS updater would install a *second* copy under
/// %LOCALAPPDATA% behind its back, so the marker turns self-updating off and
/// leaves the job to `scoop update`.
fn updates_managed() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(".managed-updates")))
        .is_some_and(|marker| marker.exists())
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<HashMap<String, String>> {
    let managed = updates_managed();
    state
        .db
        .read("get_settings", move |conn| {
            let mut map = HashMap::new();
            for key in ALLOWED {
                if let Some(v) = queries::get_setting(conn, key)? {
                    map.insert(key.to_string(), v);
                }
            }
            // Derived, not stored: read-only for the frontend (set_setting
            // rejects it, since it isn't in ALLOWED).
            if managed {
                map.insert("update_managed".to_string(), "1".to_string());
            }
            Ok(map)
        })
        .await
}

#[tauri::command]
pub async fn set_setting(state: State<'_, AppState>, key: String, value: String) -> Result<()> {
    if !ALLOWED.contains(&key.as_str()) {
        return Err(SkimError::other(
            "settings",
            format!("unknown setting: {key}"),
        ));
    }
    state
        .db
        .call(move |conn| queries::set_setting(conn, &key, &value))
        .await
}

/// Longer than this and the line is a dump, not a diagnostic.
const FRONTEND_LOG_MAX: usize = 2000;

/// A crash in the webview has nowhere to print — a windowed release build has no
/// stderr and the devtools are not there to open — so a boot that dies on an
/// unhandled rejection looks exactly like an app that simply stopped working.
/// Put the frontend's own failures next to the panic log, where they can be
/// asked for. Called from the global `error` / `unhandledrejection` handlers.
#[tauri::command]
pub fn log_frontend_error(message: String) {
    // By chars, not bytes: `String::truncate` panics mid-codepoint, and a
    // stack trace from a localized build is not guaranteed to be ASCII.
    let line: String = message
        .replace('\n', " | ")
        .chars()
        .take(FRONTEND_LOG_MAX)
        .collect();
    tracing::error!(target: "skim_lib", "frontend: {line}");
    crate::append_log("skim-frontend.log", &line);
}

/// Everything that must happen before the updater hands over to the NSIS
/// installer. The install path ends in `std::process::exit(0)` inside the
/// plugin — no unwinding, no destructors, no chance to finish a write — so the
/// app has exactly this one window to leave the machine in a state the next
/// process can boot from cleanly.
///
/// Best-effort throughout: a wedged database must never be the reason an update
/// doesn't install. The caller bounds the wait.
#[tauri::command]
pub async fn prepare_update(state: State<'_, AppState>) -> Result<()> {
    let started = std::time::Instant::now();

    // Park every engine first: no new writes means the flag below can't queue
    // behind a backfill, and no IMAP session is cut off mid-transaction.
    for handle in state.engines.lock().await.values() {
        handle.stop();
    }
    let parked_ms = started.elapsed().as_millis();

    // One-shot flag `run()` consumes on the next boot: the installer relaunches
    // Skim with the old process args (possibly autostart's `--minimized`), but
    // the user clicked "Restart" and expects the window back.
    let flagged = state
        .db
        .call(|conn| queries::set_setting(conn, "update_relaunch", "1"))
        .await
        .is_ok();
    let flagged_ms = started.elapsed().as_millis();

    // Fold the WAL back into the database. Without this the hard exit leaves
    // megabytes of journal for the freshly installed build to recover on its
    // very first query — which is the moment the user is staring at an empty
    // message list.
    let checkpointed = state
        .db
        .call(|conn| conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())))
        .await
        .is_ok();

    crate::append_log(
        "skim-update.log",
        &format!(
            "preparing v{} for install: engines parked {parked_ms}ms, flag {flagged} {flagged_ms}ms, \
             checkpoint {checkpointed} {}ms",
            env!("CARGO_PKG_VERSION"),
            started.elapsed().as_millis(),
        ),
    );
    Ok(())
}
