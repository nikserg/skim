//! Citation type and date formatting shared by the mailbox AI features.
//! (The old keyword-retrieval chat pipeline was replaced by the tool-calling
//! agent in [`crate::ai::agent`].)

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Citation {
    pub index: usize,
    pub message_id: i64,
    pub thread_id: Option<i64>,
    pub folder_id: i64,
    pub subject: String,
    pub from: String,
}

pub fn format_date(unix: i64) -> String {
    let days = unix / 86400;
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let (y, m) = if m <= 2 { (y + 1, m) } else { (y, m) };
    format!("{y}-{m:02}-{d:02}")
}
