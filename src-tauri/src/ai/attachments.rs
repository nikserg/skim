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
        let doc = lopdf::Document::load(&path).ok()?;
        pdf::extract_text(&doc, max)
    })
    .await
    .ok()
    .flatten()
}

/// Plain-text extraction from PDFs, built on lopdf primitives. Unlike lopdf's
/// own `extract_text` it turns `Td`/`TD`/`Tm` line moves into newlines instead
/// of running lines together, and it never inserts a space inside a word that
/// the generator split into several show operations for kerning.
mod pdf {
    use super::prompts;
    use lopdf::content::{Content, Operation};
    use lopdf::{Dictionary, Document, Encoding, Object, ObjectId};
    use std::collections::{BTreeMap, HashSet};

    /// Per-stream decompressed-size cap: guards against decompression bombs.
    const STREAM_LIMIT: usize = 10 * 1024 * 1024;
    /// Max nesting of Form XObjects invoked via `Do`.
    const MAX_FORM_DEPTH: usize = 8;

    /// Text of all pages in order, truncated to `max` chars. `None` when the
    /// document yields no text at all. Pages that fail to parse are skipped.
    pub fn extract_text(doc: &Document, max: usize) -> Option<String> {
        let mut out = String::new();
        for (_no, page_id) in doc.get_pages() {
            if out.chars().count() >= max {
                break;
            }
            let _ = append_page_text(doc, page_id, &mut out);
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
        }
        let trimmed = out.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(prompts::truncate(trimmed, max))
        }
    }

    fn append_page_text(doc: &Document, page_id: ObjectId, out: &mut String) -> lopdf::Result<()> {
        let fonts = doc.get_page_fonts(page_id)?;
        let encodings: BTreeMap<Vec<u8>, Encoding> = fonts
            .into_iter()
            .filter_map(|(name, font)| {
                font.get_font_encoding_with_limit(doc, STREAM_LIMIT)
                    .ok()
                    .map(|enc| (name, enc))
            })
            .collect();
        let content = Content::decode(&doc.get_page_content_with_limit(page_id, STREAM_LIMIT)?)?;

        let (resource_dict, resource_ids) = doc.get_page_resources(page_id)?;
        let mut resources: Vec<&Dictionary> = resource_dict.into_iter().collect();
        resources.extend(
            resource_ids
                .iter()
                .filter_map(|id| doc.get_dictionary(*id).ok()),
        );

        // `visited` keeps one page from pulling the same form in twice and
        // guards against reference cycles.
        let mut visited = HashSet::new();
        walk(
            doc,
            &content.operations,
            &encodings,
            &resources,
            out,
            0,
            &mut visited,
        );
        Ok(())
    }

    /// Interpret one content stream, appending its text to `out`. Recurses
    /// into Form XObjects invoked with `Do`.
    fn walk(
        doc: &Document,
        operations: &[Operation],
        encodings: &BTreeMap<Vec<u8>, Encoding>,
        resources: &[&Dictionary],
        out: &mut String,
        depth: usize,
        visited: &mut HashSet<ObjectId>,
    ) {
        let mut encoding: Option<&Encoding> = None;
        let mut font_size: f32 = 12.0;
        let mut tm_pos: Option<(f32, f32)> = None;
        // Chars shown since the line matrix last moved — a cheap stand-in for
        // the width of the text drawn on the current line segment.
        let mut seg_chars: usize = 0;
        for op in operations {
            match op.operator.as_ref() {
                "Tf" => {
                    encoding = op
                        .operands
                        .first()
                        .and_then(|o| o.as_name().ok())
                        .and_then(|name| encodings.get(name));
                    if let Some(size) = op.operands.get(1).and_then(|o| o.as_float().ok()) {
                        font_size = size.abs().max(1.0);
                    }
                }
                "Tj" | "TJ" => {
                    let before = out.len();
                    show_text(out, encoding, &op.operands);
                    seg_chars += out[before..].chars().count();
                }
                // `'` and `"` show a string on the *next* line (PDF 32000-1 §9.4.3).
                "'" => {
                    newline(out);
                    let before = out.len();
                    show_text(out, encoding, &op.operands);
                    seg_chars = out[before..].chars().count();
                }
                "\"" => {
                    newline(out);
                    let before = out.len();
                    if let Some(s) = op.operands.get(2) {
                        show_text(out, encoding, std::slice::from_ref(s));
                    }
                    seg_chars = out[before..].chars().count();
                }
                // Vertical line moves start a new line. A horizontal `Td` is a
                // kerning split inside a word (Chromium prints "Дог" Td
                // "овор") — unless it jumps clearly past anything the shown
                // chars could have covered (1 em each is a safe upper bound),
                // which makes it a column gap.
                "Td" | "TD" => {
                    let tx = op.operands.first().and_then(|o| o.as_float().ok());
                    let ty = op.operands.get(1).and_then(|o| o.as_float().ok());
                    if ty.is_some_and(|ty| ty != 0.0) {
                        newline(out);
                    } else if tx.is_some_and(|tx| tx > (seg_chars as f32 + 0.5) * font_size) {
                        space(out);
                    }
                    seg_chars = 0;
                }
                "T*" => {
                    newline(out);
                    seg_chars = 0;
                }
                // A text matrix reset on the same baseline is a new text
                // region when it jumps further than one em; smaller nudges are
                // per-glyph positioning and must not split words.
                "Tm" => {
                    let x = op.operands.get(4).and_then(|o| o.as_float().ok());
                    let y = op.operands.get(5).and_then(|o| o.as_float().ok());
                    if let (Some((px, py)), Some(x), Some(y)) = (tm_pos, x, y) {
                        if (py - y).abs() > 0.5 {
                            newline(out);
                        } else if (x - px).abs() > font_size {
                            space(out);
                        }
                    }
                    tm_pos = x.zip(y);
                    seg_chars = 0;
                }
                "BT" => {
                    tm_pos = None;
                    seg_chars = 0;
                }
                "ET" => {
                    newline(out);
                    seg_chars = 0;
                }
                // Text may live inside a Form XObject rather than the page's
                // own stream (letterheads, some invoice generators).
                "Do" if depth < MAX_FORM_DEPTH => {
                    if let Some(name) = op.operands.first().and_then(|o| o.as_name().ok()) {
                        append_form_text(doc, name, encodings, resources, out, depth, visited);
                    }
                }
                _ => {}
            }
        }
    }

    /// Extract the text of the Form XObject `name` from `resources`.
    /// Non-form XObjects (images) and unresolvable references are ignored.
    fn append_form_text(
        doc: &Document,
        name: &[u8],
        parent_encodings: &BTreeMap<Vec<u8>, Encoding>,
        parent_resources: &[&Dictionary],
        out: &mut String,
        depth: usize,
        visited: &mut HashSet<ObjectId>,
    ) {
        let Some(id) = parent_resources.iter().find_map(|res| {
            deref_dict(doc, res.get(b"XObject").ok()?)?
                .get(name)
                .ok()?
                .as_reference()
                .ok()
        }) else {
            return;
        };
        if !visited.insert(id) {
            return;
        }
        let Some(stream) = doc.get_object(id).ok().and_then(|o| o.as_stream().ok()) else {
            return;
        };
        if !matches!(
            stream.dict.get(b"Subtype").and_then(|o| o.as_name()),
            Ok(b"Form")
        ) {
            return;
        }
        let Some(content) = stream
            .decompressed_content_with_limit(STREAM_LIMIT)
            .ok()
            .and_then(|data| Content::decode(&data).ok())
        else {
            return;
        };

        // A form with its own /Resources brings its own font namespace;
        // without one it inherits the parent's (PDF 32000-1 §8.10.1).
        let own_resources = stream
            .dict
            .get(b"Resources")
            .ok()
            .and_then(|o| deref_dict(doc, o));
        match own_resources {
            Some(res) => {
                let encodings = encodings_from_resources(doc, res);
                let resources = [res];
                walk(
                    doc,
                    &content.operations,
                    &encodings,
                    &resources,
                    out,
                    depth + 1,
                    visited,
                );
            }
            None => {
                walk(
                    doc,
                    &content.operations,
                    parent_encodings,
                    parent_resources,
                    out,
                    depth + 1,
                    visited,
                );
            }
        }
    }

    fn encodings_from_resources<'a>(
        doc: &'a Document,
        resources: &'a Dictionary,
    ) -> BTreeMap<Vec<u8>, Encoding<'a>> {
        let Some(fonts) = resources.get(b"Font").ok().and_then(|o| deref_dict(doc, o)) else {
            return BTreeMap::new();
        };
        fonts
            .iter()
            .filter_map(|(name, obj)| {
                deref_dict(doc, obj)?
                    .get_font_encoding_with_limit(doc, STREAM_LIMIT)
                    .ok()
                    .map(|enc| (name.clone(), enc))
            })
            .collect()
    }

    fn deref_dict<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a Dictionary> {
        doc.dereference(obj).ok()?.1.as_dict().ok()
    }

    /// Append the text of a Tj/TJ operand list. In TJ arrays, a large negative
    /// kerning adjustment (< -100/1000 em) is treated as a word gap.
    fn show_text(out: &mut String, encoding: Option<&Encoding>, operands: &[Object]) {
        let Some(enc) = encoding else { return };
        for op in operands {
            match op {
                Object::String(bytes, _) => {
                    let _ = enc.write_to_string(bytes, out);
                }
                Object::Array(arr) => show_text(out, encoding, arr),
                _ => {
                    if op.as_float().is_ok_and(|n| n < -100.0) {
                        space(out);
                    }
                }
            }
        }
    }

    fn newline(out: &mut String) {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
    }

    fn space(out: &mut String) {
        if !out.is_empty() && !out.ends_with(char::is_whitespace) {
            out.push(' ');
        }
    }
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
    fn extracts_pdf_text_with_line_breaks() {
        use lopdf::content::{Content, Operation};
        use lopdf::{dictionary, Document, Object, Stream};

        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new("Td", vec![50.into(), 700.into()]),
                Operation::new("Tj", vec![Object::string_literal("Hello Skim")]),
                // Horizontal kerning split must not become a space…
                Operation::new("Td", vec![30.into(), 0.into()]),
                Operation::new("Tj", vec![Object::string_literal("!")]),
                // …a horizontal jump past the drawn text is a column gap…
                Operation::new("Td", vec![200.into(), 0.into()]),
                Operation::new("Tj", vec![Object::string_literal("Col2")]),
                // …and a vertical move is a line break.
                Operation::new("T*", vec![]),
                Operation::new("Tj", vec![Object::string_literal("Second line")]),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
            }),
        );
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);

        assert_eq!(
            pdf::extract_text(&doc, 100).as_deref(),
            Some("Hello Skim! Col2\nSecond line")
        );
        // Truncation respects the char budget.
        assert_eq!(
            pdf::extract_text(&doc, 5).as_deref(),
            Some("Hello\n…(truncated)")
        );
    }

    #[test]
    fn extracts_text_from_form_xobjects() {
        use lopdf::content::{Content, Operation};
        use lopdf::{dictionary, Document, Object, Stream};

        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        // Letterhead-style form with its own resources.
        let form_content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F9".into(), 10.into()]),
                Operation::new("Tj", vec![Object::string_literal("Letterhead")]),
                Operation::new("ET", vec![]),
            ],
        };
        let form_id = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Form",
                "Resources" => dictionary! {
                    "Font" => dictionary! { "F9" => font_id },
                },
            },
            form_content.encode().unwrap(),
        ));
        let content = Content {
            operations: vec![
                Operation::new("Do", vec!["X1".into()]),
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new("Tj", vec![Object::string_literal("Body")]),
                Operation::new("ET", vec![]),
                // Re-invoking the same form must not duplicate its text.
                Operation::new("Do", vec!["X1".into()]),
            ],
        };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
            "XObject" => dictionary! { "X1" => form_id },
        });
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
            }),
        );
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);

        assert_eq!(
            pdf::extract_text(&doc, 100).as_deref(),
            Some("Letterhead\nBody")
        );
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
