//! Conversion of fetched IMAP data into database rows via `mail-parser`.

use crate::db::models::{Address, NewMessage};
use mail_parser::{Addr, HeaderName, HeaderValue, MessageParser, MimeHeaders};

fn convert_addr(a: &Addr) -> Option<Address> {
    a.address.as_ref().map(|addr| Address {
        name: a.name.as_ref().map(|n| n.to_string()),
        addr: addr.to_string(),
    })
}

fn convert_addrs(value: &mail_parser::Address) -> Vec<Address> {
    match value {
        mail_parser::Address::List(list) => list.iter().filter_map(convert_addr).collect(),
        mail_parser::Address::Group(groups) => groups
            .iter()
            .flat_map(|g| g.addresses.iter())
            .filter_map(convert_addr)
            .collect(),
    }
}

/// Parse fetched header bytes into a `NewMessage`. `internal_date` (unix
/// seconds) is the fallback when the Date header is missing or invalid.
#[allow(clippy::too_many_arguments)] // flat FETCH attributes, one call site
pub fn parse_headers(
    account_id: &str,
    folder_id: i64,
    uid: u32,
    header_bytes: &[u8],
    internal_date: Option<i64>,
    size: Option<u32>,
    is_read: bool,
    is_starred: bool,
    has_attachments: bool,
) -> NewMessage {
    let parsed = MessageParser::default().parse_headers(header_bytes);

    let mut msg = NewMessage {
        account_id: account_id.to_string(),
        folder_id,
        uid,
        size: size.map(|s| s as i64),
        is_read,
        is_starred,
        has_attachments,
        date: internal_date.unwrap_or(0),
        ..Default::default()
    };

    let Some(parsed) = parsed else {
        return msg;
    };

    msg.subject = parsed.subject().map(|s| s.to_string());
    msg.message_id = parsed.message_id().map(|s| format!("<{s}>"));

    if let Some(date) = parsed.date() {
        let ts = date.to_timestamp();
        if ts > 0 {
            msg.date = ts;
        }
    }

    if let Some(from) = parsed.from().and_then(|a| match a {
        mail_parser::Address::List(l) => l.first(),
        mail_parser::Address::Group(g) => g.first().and_then(|g| g.addresses.first()),
    }) {
        msg.from_name = from.name.as_ref().map(|n| n.to_string());
        msg.from_addr = from.address.as_ref().map(|a| a.to_string());
    }

    if let Some(to) = parsed.to() {
        msg.to_addrs = convert_addrs(to);
    }
    if let Some(cc) = parsed.cc() {
        msg.cc_addrs = convert_addrs(cc);
    }

    // In-Reply-To / References come back as text or text lists.
    for header in parsed.headers() {
        if header.name().eq_ignore_ascii_case("In-Reply-To") {
            if let Some(first) = header_text_list(header.value()).into_iter().next() {
                msg.in_reply_to = Some(format!("<{first}>"));
            }
        } else if header.name().eq_ignore_ascii_case("References") {
            msg.references = header_text_list(header.value())
                .into_iter()
                .map(|s| format!("<{s}>"))
                .collect();
        }
    }

    // mail-parser types List-Unsubscribe as an address (the `<...>` brackets),
    // which drops the mailto:/https: scheme. Read the RAW header instead so the
    // full `<uri>, <uri>` list survives for the unsubscribe command to parse.
    msg.list_unsubscribe = parsed
        .header_raw(HeaderName::ListUnsubscribe)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if let Some(post) = parsed.header_raw("List-Unsubscribe-Post") {
        // Presence of "One-Click" opts the list into RFC 8058 one-click POST.
        if post.to_ascii_lowercase().contains("one-click") {
            msg.list_unsubscribe_one_click = true;
        }
    }

    msg
}

fn header_text_list(value: &HeaderValue) -> Vec<String> {
    match value {
        HeaderValue::Text(t) => vec![t.to_string()],
        HeaderValue::TextList(l) => l.iter().map(|t| t.to_string()).collect(),
        _ => vec![],
    }
}

/// Extract plain-text + html bodies and a snippet from a full RFC822 payload.
pub struct ParsedBody {
    pub text: Option<String>,
    pub html: Option<String>,
    pub snippet: String,
    pub attachments: Vec<ParsedAttachment>,
}

pub struct ParsedAttachment {
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub size: i64,
    pub content_id: Option<String>,
    pub is_inline: bool,
    pub data: Vec<u8>,
}

pub fn parse_body(raw: &[u8]) -> ParsedBody {
    let Some(parsed) = MessageParser::default().parse(raw) else {
        return ParsedBody {
            text: None,
            html: None,
            snippet: String::new(),
            attachments: vec![],
        };
    };

    let html = parsed.body_html(0).map(|s| s.to_string());
    let text = parsed
        .body_text(0)
        .map(|s| s.to_string())
        .or_else(|| html.as_deref().map(html_to_text));

    let snippet = text.as_deref().map(make_snippet).unwrap_or_default();

    let attachments = parsed
        .attachments()
        .map(|part| {
            let content_type = part.content_type();
            ParsedAttachment {
                filename: part.attachment_name().map(|s| s.to_string()),
                mime_type: content_type.map(|ct| match ct.subtype() {
                    Some(sub) => format!("{}/{}", ct.ctype(), sub),
                    None => ct.ctype().to_string(),
                }),
                size: part.contents().len() as i64,
                content_id: part.content_id().map(|s| s.to_string()),
                is_inline: part
                    .content_disposition()
                    .is_none_or(|d| !d.is_attachment()),
                data: part.contents().to_vec(),
            }
        })
        .collect();

    ParsedBody {
        text,
        html,
        snippet,
        attachments,
    }
}

/// Cheap HTML → text for FTS/snippets when a message has no text part.
pub fn html_to_text(html: &str) -> String {
    // Tag names are ASCII, so compare bytes case-insensitively in place —
    // a `to_lowercase()` copy has different byte offsets (`İ` grows), and
    // indexing the original's offsets into it can split a UTF-8 char.
    fn starts_ignore_ascii_case(s: &str, prefix: &str) -> bool {
        s.len() >= prefix.len()
            && s.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
    }

    let mut out = String::with_capacity(html.len() / 2);
    let mut in_tag = false;
    let mut in_script = false;
    let mut i = 0;
    let bytes = html.as_bytes();
    while i < bytes.len() {
        if !in_tag && bytes[i] == b'<' {
            in_tag = true;
            let rest = &html[i..];
            if starts_ignore_ascii_case(rest, "<script") || starts_ignore_ascii_case(rest, "<style")
            {
                in_script = true;
            } else if in_script
                && (starts_ignore_ascii_case(rest, "</script")
                    || starts_ignore_ascii_case(rest, "</style"))
            {
                in_script = false;
            }
        } else if in_tag && bytes[i] == b'>' {
            in_tag = false;
        } else if !in_tag && !in_script {
            // SAFETY: outside tags `i` always sits on a char boundary — it
            // advances by whole chars here and past ASCII `<`/`>` elsewhere.
            let ch = html[i..].chars().next().unwrap_or(' ');
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        i += 1;
    }
    // Decode the handful of entities that matter for readability.
    let out = out
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A `<uri>`-bracketed target extracted from a `List-Unsubscribe` header.
pub enum UnsubTarget {
    /// An http(s): endpoint. Only `https:` ones may receive an RFC 8058
    /// one-click POST; plain `http:` is only ever opened in the browser.
    Http(String),
    /// A mailto: address plus its optional `?subject=`.
    Mail { to: String, subject: String },
}

/// Parse a raw `List-Unsubscribe` value (`<uri>, <uri>`) into its targets.
pub fn parse_unsub_targets(raw: &str) -> Vec<UnsubTarget> {
    let mut out = Vec::new();
    for part in raw.split(',') {
        let uri = part
            .trim()
            .trim_start_matches('<')
            .trim_end_matches('>')
            .trim();
        if let Some(rest) = uri.strip_prefix("mailto:") {
            let (addr, query) = rest.split_once('?').unwrap_or((rest, ""));
            let addr = addr.trim();
            if addr.is_empty() {
                continue;
            }
            let subject = query
                .split('&')
                .find_map(|kv| kv.strip_prefix("subject="))
                .map(percent_decode)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "unsubscribe".to_string());
            out.push(UnsubTarget::Mail {
                to: addr.to_string(),
                subject,
            });
        } else if uri.starts_with("https://") || uri.starts_with("http://") {
            out.push(UnsubTarget::Http(uri.to_string()));
        }
    }
    out
}

/// Minimal `application/x-www-form-urlencoded` decode for a mailto `subject`.
fn percent_decode(s: &str) -> String {
    let bytes = s.replace('+', " ").into_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&String::from_utf8_lossy(&bytes[i + 1..i + 3]), 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The domain an unsubscribe click will actually contact, shown next to the
/// chip so the user sees the destination before acting. Mirrors the action
/// priority of `commands::mail::unsubscribe`: one-click https POST →
/// unsubscribe email → link opened in the browser.
pub fn unsubscribe_host(raw: &str, one_click: bool) -> Option<String> {
    let targets = parse_unsub_targets(raw);
    let https_host = targets.iter().find_map(|t| match t {
        UnsubTarget::Http(u) if u.starts_with("https://") => {
            url::Url::parse(u).ok()?.host_str().map(str::to_string)
        }
        _ => None,
    });
    if one_click {
        if let Some(host) = https_host.clone() {
            return Some(host);
        }
    }
    let mail_host = targets.iter().find_map(|t| match t {
        UnsubTarget::Mail { to, .. } => to.rsplit_once('@').map(|(_, d)| d.to_string()),
        _ => None,
    });
    mail_host.or(https_host).or_else(|| {
        targets.iter().find_map(|t| match t {
            UnsubTarget::Http(u) => url::Url::parse(u).ok()?.host_str().map(str::to_string),
            _ => None,
        })
    })
}

pub fn make_snippet(text: &str) -> String {
    let cleaned = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut snippet: String = cleaned.chars().take(140).collect();
    if cleaned.chars().count() > 140 {
        snippet.push('…');
    }
    snippet
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(raw: &str) -> NewMessage {
        parse_headers(
            "acct",
            1,
            42,
            raw.as_bytes(),
            None,
            None,
            false,
            false,
            false,
        )
    }

    #[test]
    fn extracts_list_unsubscribe_with_one_click() {
        let msg = headers(
            "From: News <news@example.com>\r\n\
             Subject: Weekly\r\n\
             List-Unsubscribe: <mailto:unsub@example.com?subject=stop>, <https://example.com/u?t=abc>\r\n\
             List-Unsubscribe-Post: List-Unsubscribe=One-Click\r\n\
             \r\n",
        );
        let raw = msg.list_unsubscribe.expect("header should be captured");
        assert!(raw.contains("mailto:unsub@example.com"));
        assert!(raw.contains("https://example.com/u?t=abc"));
        assert!(msg.list_unsubscribe_one_click);
    }

    #[test]
    fn list_unsubscribe_without_post_is_not_one_click() {
        let msg = headers(
            "From: News <news@example.com>\r\n\
             List-Unsubscribe: <https://example.com/u>\r\n\
             \r\n",
        );
        assert!(msg.list_unsubscribe.is_some());
        assert!(!msg.list_unsubscribe_one_click);
    }

    #[test]
    fn no_list_header_means_no_unsubscribe() {
        let msg = headers("From: A Friend <friend@example.com>\r\nSubject: Hi\r\n\r\n");
        assert!(msg.list_unsubscribe.is_none());
        assert!(!msg.list_unsubscribe_one_click);
    }

    #[test]
    fn html_to_text_survives_length_changing_unicode() {
        // 'İ' lowercases to two chars — a lowercased copy has shifted byte
        // offsets, which used to desync and panic on a mid-char slice.
        let html = "İİİİ<script>var x = 1;</script><b>текст</b> ürün";
        assert_eq!(html_to_text(html), "İİİİтекст ürün");
    }

    #[test]
    fn html_to_text_strips_uppercase_script_and_style() {
        let html = "<SCRIPT>alert(1)</SCRIPT>hello <STYLE>b{}</STYLE>world";
        assert_eq!(html_to_text(html), "hello world");
    }

    #[test]
    fn unsubscribe_host_prefers_one_click_https() {
        let raw = "<mailto:unsub@lists.example.org>, <https://mail.example.com/u?t=1>";
        assert_eq!(
            unsubscribe_host(raw, true).as_deref(),
            Some("mail.example.com")
        );
        // Without one-click the mail path wins, matching the command's priority.
        assert_eq!(
            unsubscribe_host(raw, false).as_deref(),
            Some("lists.example.org")
        );
        assert_eq!(
            unsubscribe_host("<https://mail.example.com/u>", false).as_deref(),
            Some("mail.example.com")
        );
        assert_eq!(unsubscribe_host("junk", true), None);
    }
}
