//! Collect an email's attachments for the AI features. Text-bearing files
//! (and PDFs on non-Anthropic providers) are extracted to text and spliced
//! into the prompt; PDFs and images on Anthropic are handed over natively as
//! `document` / `image` content blocks. A shared [`Budget`] bounds how much is
//! pulled in so requests stay reasonable.

use super::{prompts, MediaBlock, MediaKind};
use crate::db::{bodies, Db};
use base64::Engine;

/// Total extracted-text characters across all attachments of one request.
const TEXT_TOTAL: usize = 12_000;
/// Per-attachment extracted-text cap.
const TEXT_PER_FILE: usize = 6_000;
/// Max native media files (PDFs + images) per request.
const MEDIA_FILES: usize = 5;
/// Max combined size of native media files (decoded bytes) per request.
const MEDIA_BYTES_TOTAL: u64 = 15 * 1024 * 1024;
/// Per-file native media size cap.
const MEDIA_BYTES_PER_FILE: u64 = 10 * 1024 * 1024;

/// Shared, request-wide budget. Threaded across the messages of a chain so the
/// anchor (open) message — processed first — always gets its share.
pub struct Budget {
    text_chars: usize,
    media_files: usize,
    media_bytes: u64,
}

impl Default for Budget {
    fn default() -> Self {
        Budget {
            text_chars: TEXT_TOTAL,
            media_files: MEDIA_FILES,
            media_bytes: MEDIA_BYTES_TOTAL,
        }
    }
}

/// The outcome of collecting one message's attachments.
#[derive(Default)]
pub struct Collected {
    /// Notes + extracted text to append to the message's `EmailBlock`.
    pub notes: String,
    /// Native media blocks (Anthropic only).
    pub media: Vec<MediaBlock>,
}

/// Gather the non-inline attachments of `message_id`. `native_media` is true
/// for Anthropic (PDFs/images go over as content blocks); otherwise everything
/// falls back to local text extraction. Best-effort: unreadable files become a
/// short note rather than an error.
pub async fn collect_for_message(
    db: &Db,
    message_id: i64,
    native_media: bool,
    budget: &mut Budget,
) -> Collected {
    let files = match db
        .call(move |conn| bodies::list_attachment_files(conn, message_id))
        .await
    {
        Ok(files) => files,
        Err(_) => return Collected::default(),
    };

    let mut out = Collected::default();
    let mut lines: Vec<String> = Vec::new();
    for f in files {
        let name = f.filename.clone().unwrap_or_else(|| "attachment".into());
        let mime = f.mime_type.clone().unwrap_or_default();

        let Some(path) = f.cache_path.clone() else {
            lines.push(format!("- \"{name}\" — not downloaded yet."));
            continue;
        };

        let kind = classify(&mime, &name);
        match kind {
            Kind::Pdf => {
                if native_media && fits_media(budget, f.size) {
                    if let Some(data) = read_base64(&path).await {
                        budget.media_files -= 1;
                        budget.media_bytes =
                            budget.media_bytes.saturating_sub(f.size.max(0) as u64);
                        out.media.push(MediaBlock {
                            kind: MediaKind::Pdf,
                            media_type: "application/pdf".into(),
                            data_base64: data,
                            filename: name.clone(),
                        });
                        lines.push(format!(
                            "- \"{name}\" (PDF) — provided to you as a document below."
                        ));
                        continue;
                    }
                }
                // Fallback: extract text locally.
                match extract_pdf_text(&path, budget.text_chars.min(TEXT_PER_FILE)).await {
                    Some(text) if !text.trim().is_empty() => {
                        budget.text_chars = budget.text_chars.saturating_sub(text.chars().count());
                        lines.push(format!("- \"{name}\" (PDF), extracted text:\n{text}"));
                    }
                    _ => lines.push(format!(
                        "- \"{name}\" (PDF) — content couldn't be extracted."
                    )),
                }
            }
            Kind::Image => {
                if native_media && fits_media(budget, f.size) {
                    if let Some(data) = read_base64(&path).await {
                        budget.media_files -= 1;
                        budget.media_bytes =
                            budget.media_bytes.saturating_sub(f.size.max(0) as u64);
                        out.media.push(MediaBlock {
                            kind: MediaKind::Image,
                            media_type: image_media_type(&mime, &name),
                            data_base64: data,
                            filename: name.clone(),
                        });
                        lines.push(format!(
                            "- \"{name}\" (image) — provided to you as an image below."
                        ));
                        continue;
                    }
                }
                lines.push(format!(
                    "- \"{name}\" (image) — not readable with the current AI provider."
                ));
            }
            Kind::Text => match read_text(&path, budget.text_chars.min(TEXT_PER_FILE)).await {
                Some(text) if !text.trim().is_empty() => {
                    budget.text_chars = budget.text_chars.saturating_sub(text.chars().count());
                    lines.push(format!("- \"{name}\" ({mime}), contents:\n{text}"));
                }
                _ => lines.push(format!("- \"{name}\" ({mime}) — empty or unreadable.")),
            },
            Kind::Other => {
                let ty = if mime.is_empty() {
                    "unknown type"
                } else {
                    &mime
                };
                lines.push(format!(
                    "- \"{name}\" ({ty}) — content couldn't be extracted."
                ));
            }
        }
    }

    out.notes = lines.join("\n");
    out
}

enum Kind {
    Pdf,
    Image,
    Text,
    Other,
}

fn ext(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

fn classify(mime: &str, name: &str) -> Kind {
    let e = ext(name);
    if mime == "application/pdf" || e == "pdf" {
        return Kind::Pdf;
    }
    if mime.starts_with("image/") || matches!(e.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp") {
        return Kind::Image;
    }
    let texty_mime = mime.starts_with("text/")
        || matches!(
            mime,
            "application/json" | "application/xml" | "application/csv"
        );
    let texty_ext = matches!(
        e.as_str(),
        "txt" | "md" | "markdown" | "csv" | "tsv" | "json" | "log" | "xml" | "yml" | "yaml" | "ini"
    );
    if texty_mime || texty_ext {
        return Kind::Text;
    }
    Kind::Other
}

fn image_media_type(mime: &str, name: &str) -> String {
    if mime.starts_with("image/") {
        return mime.to_string();
    }
    match ext(name).as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "image/png",
    }
    .to_string()
}

fn fits_media(budget: &Budget, size: i64) -> bool {
    let size = size.max(0) as u64;
    budget.media_files > 0 && size <= MEDIA_BYTES_PER_FILE && size <= budget.media_bytes
}

/// Read a file and base64-encode it, off the async runtime. `None` on any error.
async fn read_base64(path: &str) -> Option<String> {
    let path = path.to_string();
    tokio::task::spawn_blocking(move || {
        std::fs::read(&path)
            .ok()
            .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes))
    })
    .await
    .ok()
    .flatten()
}

/// Read a text file as UTF-8 (lossy), truncated to `max` chars.
async fn read_text(path: &str, max: usize) -> Option<String> {
    if max == 0 {
        return None;
    }
    let path = path.to_string();
    tokio::task::spawn_blocking(move || {
        std::fs::read(&path)
            .ok()
            .map(|bytes| prompts::truncate(&String::from_utf8_lossy(&bytes), max))
    })
    .await
    .ok()
    .flatten()
}

/// Extract text from a PDF, truncated to `max` chars. `None` on any error.
async fn extract_pdf_text(path: &str, max: usize) -> Option<String> {
    if max == 0 {
        return None;
    }
    let path = path.to_string();
    tokio::task::spawn_blocking(move || {
        // pdf-extract can panic on some malformed PDFs; contain it.
        std::panic::catch_unwind(|| pdf_extract::extract_text(&path).ok())
            .ok()
            .flatten()
            .map(|text| prompts::truncate(&text, max))
    })
    .await
    .ok()
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(mime: &str, name: &str) -> &'static str {
        match classify(mime, name) {
            Kind::Pdf => "pdf",
            Kind::Image => "image",
            Kind::Text => "text",
            Kind::Other => "other",
        }
    }

    #[test]
    fn classifies_by_mime_and_extension() {
        assert_eq!(kind("application/pdf", "x.bin"), "pdf");
        assert_eq!(kind("", "Meeting.pdf"), "pdf");
        assert_eq!(kind("image/jpeg", "x"), "image");
        assert_eq!(kind("", "photo.PNG"), "image");
        assert_eq!(kind("text/plain", "notes"), "text");
        assert_eq!(kind("", "data.csv"), "text");
        assert_eq!(kind("application/json", "x"), "text");
        assert_eq!(kind("application/vnd.ms-excel", "sheet.xlsx"), "other");
        assert_eq!(kind("application/zip", "a.zip"), "other");
    }

    #[test]
    fn image_media_type_falls_back_to_extension() {
        assert_eq!(image_media_type("image/webp", "x"), "image/webp");
        assert_eq!(image_media_type("", "a.jpg"), "image/jpeg");
        assert_eq!(image_media_type("", "a.gif"), "image/gif");
        assert_eq!(image_media_type("", "a.unknown"), "image/png");
    }

    #[test]
    fn fits_media_respects_caps() {
        let budget = Budget::default();
        assert!(fits_media(&budget, 1_000));
        assert!(!fits_media(&budget, (MEDIA_BYTES_PER_FILE + 1) as i64));
        let empty = Budget {
            media_files: 0,
            ..Budget::default()
        };
        assert!(!fits_media(&empty, 1));
    }
}
