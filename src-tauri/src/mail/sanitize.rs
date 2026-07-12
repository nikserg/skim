//! HTML email sanitization. Runs in Rust so unsanitized markup never crosses
//! the IPC boundary; the frontend additionally renders the result inside a
//! sandboxed iframe with a strict CSP.

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub struct SanitizedHtml {
    pub html: String,
    pub blocked_images: usize,
}

/// CSS properties allowed in inline `style` attributes.
const STYLE_ALLOWED: &[&str] = &[
    "color",
    "background",
    "background-color",
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "text-align",
    "text-decoration",
    "line-height",
    "letter-spacing",
    "margin",
    "margin-top",
    "margin-bottom",
    "margin-left",
    "margin-right",
    "padding",
    "padding-top",
    "padding-bottom",
    "padding-left",
    "padding-right",
    "border",
    "border-top",
    "border-bottom",
    "border-left",
    "border-right",
    "border-radius",
    "border-collapse",
    "border-spacing",
    "width",
    "max-width",
    "min-width",
    "height",
    "display",
    "vertical-align",
    "white-space",
    "word-break",
    "overflow-wrap",
];

fn filter_style(value: &str) -> Option<String> {
    let mut kept = Vec::new();
    for decl in value.split(';') {
        let Some((prop, val)) = decl.split_once(':') else {
            continue;
        };
        let prop = prop.trim().to_lowercase();
        let val = val.trim();
        let val_lower = val.to_lowercase();
        if !STYLE_ALLOWED.contains(&prop.as_str()) {
            continue;
        }
        // No external fetches or layout escapes through CSS.
        if val_lower.contains("url(")
            || val_lower.contains("expression(")
            || val_lower.contains("fixed")
            || val_lower.contains("absolute")
            || val_lower.contains("important")
        {
            continue;
        }
        kept.push(format!("{prop}:{val}"));
    }
    if kept.is_empty() {
        None
    } else {
        Some(kept.join(";"))
    }
}

/// Resolve a `cid:` reference to the URL served by the `skim-cid` protocol.
/// (On Windows, Tauri custom protocols are exposed as `http://<scheme>.localhost/`.)
pub fn cid_url(message_id: i64, content_id: &str) -> String {
    format!(
        "http://skim-cid.localhost/{message_id}/{}",
        urlencode(content_id)
    )
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Sanitize an HTML email body.
///
/// * `message_id` — used to resolve `cid:` inline images.
/// * `allow_remote_images` — when false, http(s) image sources are emptied
///   and counted so the UI can offer a "show images" bar.
pub fn sanitize_email_html(
    html: &str,
    message_id: i64,
    allow_remote_images: bool,
) -> SanitizedHtml {
    let blocked = Arc::new(AtomicUsize::new(0));
    let blocked_in_filter = blocked.clone();

    let tags: HashSet<&str> = [
        "a",
        "abbr",
        "b",
        "blockquote",
        "br",
        "caption",
        "center",
        "code",
        "col",
        "colgroup",
        "dd",
        "div",
        "dl",
        "dt",
        "em",
        "font",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "hr",
        "i",
        "img",
        "li",
        "ol",
        "p",
        "pre",
        "q",
        "s",
        "small",
        "span",
        "strike",
        "strong",
        "sub",
        "sup",
        "table",
        "tbody",
        "td",
        "tfoot",
        "th",
        "thead",
        "tr",
        "u",
        "ul",
    ]
    .into();

    let mut builder = ammonia::Builder::empty();
    builder
        .tags(tags)
        .clean_content_tags(["script", "style", "svg", "math", "head", "title"].into())
        .generic_attributes(["style", "align", "valign", "dir"].into())
        .add_tag_attributes("a", ["href", "title"])
        .add_tag_attributes("img", ["src", "alt", "width", "height", "border"])
        .add_tag_attributes("td", ["colspan", "rowspan", "width", "height", "bgcolor"])
        .add_tag_attributes("th", ["colspan", "rowspan", "width", "height", "bgcolor"])
        .add_tag_attributes(
            "table",
            ["cellpadding", "cellspacing", "border", "width", "bgcolor"],
        )
        .add_tag_attributes("font", ["color", "size", "face"])
        // `cid`/`data` are allowed through so the img filter below can see
        // them; anchors are re-restricted to http/https/mailto in the filter.
        .url_schemes(["http", "https", "mailto", "cid", "data"].into())
        .link_rel(Some("noopener noreferrer"))
        .attribute_filter(move |element, attribute, value| {
            if attribute == "style" {
                return filter_style(value).map(Cow::Owned);
            }
            if element == "a" && attribute == "href" {
                let lower = value.trim().to_lowercase();
                if lower.starts_with("http://")
                    || lower.starts_with("https://")
                    || lower.starts_with("mailto:")
                {
                    return Some(Cow::Borrowed(value));
                }
                return None;
            }
            if element == "img" && attribute == "src" {
                let lower = value.trim().to_lowercase();
                if let Some(cid) = lower.strip_prefix("cid:") {
                    return Some(Cow::Owned(cid_url(message_id, cid.trim())));
                }
                if lower.starts_with("data:image/") {
                    return Some(Cow::Owned(value.to_string()));
                }
                if lower.starts_with("http://") || lower.starts_with("https://") {
                    if allow_remote_images {
                        return Some(Cow::Owned(value.to_string()));
                    }
                    blocked_in_filter.fetch_add(1, Ordering::Relaxed);
                    return Some(Cow::Borrowed(""));
                }
                return None;
            }
            Some(Cow::Borrowed(value))
        });

    let html = builder.clean(html).to_string();
    SanitizedHtml {
        html,
        blocked_images: blocked.load(Ordering::Relaxed),
    }
}

/// Render a plain-text body as safe HTML: escape, linkify, preserve wrapping.
pub fn text_to_html(text: &str) -> String {
    let escaped = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let linkified = linkify(&escaped);
    format!("<pre class=\"skim-plain\">{linkified}</pre>")
}

fn linkify(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find("http") {
        let (before, tail) = rest.split_at(pos);
        out.push_str(before);
        if tail.starts_with("http://") || tail.starts_with("https://") {
            let end = tail
                .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '<')
                .unwrap_or(tail.len());
            let (url, after) = tail.split_at(end);
            let trimmed = url.trim_end_matches(['.', ',', ')', ']', ';']);
            let extra = &url[trimmed.len()..];
            out.push_str(&format!(
                "<a href=\"{trimmed}\" rel=\"noopener noreferrer\">{trimmed}</a>{extra}"
            ));
            rest = after;
        } else {
            out.push_str("http");
            rest = &tail[4..];
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_scripts_and_event_handlers() {
        let s = sanitize_email_html(
            "<p onclick=\"evil()\">hi<script>alert(1)</script></p><style>body{}</style>",
            1,
            true,
        );
        assert!(!s.html.contains("script"));
        assert!(!s.html.contains("onclick"));
        assert!(!s.html.contains("body{}"));
        assert!(s.html.contains("hi"));
    }

    #[test]
    fn blocks_remote_images_and_counts() {
        let s = sanitize_email_html(
            "<img src=\"https://tracker.example/x.png\"><img src=\"data:image/png;base64,AA==\">",
            1,
            false,
        );
        assert_eq!(s.blocked_images, 1);
        assert!(!s.html.contains("tracker.example"));
        assert!(s.html.contains("data:image/png"));
    }

    #[test]
    fn rewrites_cid_references() {
        let s = sanitize_email_html("<img src=\"cid:logo@corp\">", 42, false);
        assert_eq!(s.blocked_images, 0);
        assert!(s.html.contains("http://skim-cid.localhost/42/logo%40corp"));
    }

    #[test]
    fn style_filter_drops_dangerous_values() {
        assert_eq!(
            filter_style("color: red; background: url(https://x); position: absolute"),
            Some("color:red".to_string())
        );
        assert_eq!(filter_style("position: fixed"), None);
    }

    #[test]
    fn javascript_links_removed() {
        let s = sanitize_email_html("<a href=\"javascript:alert(1)\">x</a>", 1, true);
        assert!(!s.html.contains("javascript"));
    }

    #[test]
    fn plain_text_is_escaped_and_linkified() {
        let html = text_to_html("see <b> https://example.com/a.");
        assert!(html.contains("&lt;b&gt;"));
        assert!(html.contains("<a href=\"https://example.com/a\""));
    }
}
