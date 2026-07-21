use crate::db::Db;
use crate::mail::sync::SyncHandle;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::Mutex;

pub struct AppState {
    pub db: Db,
    pub data_dir: PathBuf,
    pub engines: Mutex<HashMap<String, SyncHandle>>,
    /// In-flight AI requests, cancellable by request id.
    pub ai_tasks: std::sync::Mutex<HashMap<String, tokio::task::AbortHandle>>,
    /// Thread to open once the frontend is ready, set when the app is
    /// cold-launched by a `skim://open` toast click.
    /// `(folder_id, thread_id, message_id)`.
    pub pending_open: std::sync::Mutex<Option<(i64, i64, i64)>>,
}

impl AppState {
    pub fn new(db: Db, data_dir: PathBuf) -> Self {
        Self {
            db,
            data_dir,
            engines: Mutex::new(HashMap::new()),
            ai_tasks: std::sync::Mutex::new(HashMap::new()),
            pending_open: std::sync::Mutex::new(None),
        }
    }
}
