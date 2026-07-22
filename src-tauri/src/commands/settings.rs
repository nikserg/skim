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
    "palette_expanded",
    "autostart",
    "ai_model",
    "ai_provider",
    "openrouter_model",
    "ai_user_name",
    "ai_style",
    "ai_style_profile",
    "ai_instructions",
    "update_last_check",
    "update_dismissed",
    "update_relaunch",
];

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<HashMap<String, String>> {
    state
        .db
        .call(|conn| {
            let mut map = HashMap::new();
            for key in ALLOWED {
                if let Some(v) = queries::get_setting(conn, key)? {
                    map.insert(key.to_string(), v);
                }
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
