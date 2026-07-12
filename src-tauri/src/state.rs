use crate::db::Db;
use crate::mail::sync::SyncHandle;
use std::collections::HashMap;
use tokio::sync::Mutex;

pub struct AppState {
    pub db: Db,
    pub engines: Mutex<HashMap<String, SyncHandle>>,
}

impl AppState {
    pub fn new(db: Db) -> Self {
        Self {
            db,
            engines: Mutex::new(HashMap::new()),
        }
    }
}
