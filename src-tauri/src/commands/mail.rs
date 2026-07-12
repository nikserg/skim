use crate::db::models::{Folder, ThreadRow};
use crate::db::queries;
use crate::error::Result;
use crate::state::AppState;
use tauri::State;

#[tauri::command]
pub async fn list_folders(state: State<'_, AppState>, account_id: String) -> Result<Vec<Folder>> {
    state
        .db
        .call(move |conn| queries::list_folders(conn, &account_id))
        .await
}

#[tauri::command]
pub async fn list_threads(
    state: State<'_, AppState>,
    folder_id: i64,
    offset: i64,
    limit: i64,
) -> Result<Vec<ThreadRow>> {
    state
        .db
        .call(move |conn| queries::list_threads(conn, folder_id, offset, limit.clamp(1, 200)))
        .await
}

#[tauri::command]
pub async fn sync_now(state: State<'_, AppState>, account_id: Option<String>) -> Result<()> {
    let engines = state.engines.lock().await;
    match account_id {
        Some(id) => {
            if let Some(handle) = engines.get(&id) {
                handle.sync_all();
            }
        }
        None => {
            for handle in engines.values() {
                handle.sync_all();
            }
        }
    }
    Ok(())
}
