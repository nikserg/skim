//! Conversion of fetched IMAP data into database rows via `mail-parser`.

use crate::db::models::{Address, NewMessage};
use mail_parser::{Addr, HeaderName, HeaderValue, MessageParser, MimeHeaders, PartType};

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

    if let Some(reply_to) = parsed.reply_to().and_then(|a| match a {
        mail_parser::Address::List(l) => l.first(),
        mail_parser::Address::Group(g) => g.first().and_then(|g| g.addresses.first()),
    }) {
        msg.reply_to_addr = reply_to.address.as_ref().map(|a| a.to_string());
    }

    // In-Reply-To / References come back as text or text lists.
    let mut saw_auth_results = false;
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
        } else if !saw_auth_results && header.name().eq_ignore_ascii_case("Authentication-Results")
        {
            // Only the topmost occurrence: that's the one our own receiving
            // server added; lower ones may come from forwarders — or from the
            // sender, which is exactly what a phisher would forge. Read the
            // raw byte range so header parsing quirks can't drop the value.
            saw_auth_results = true;
            let start = header.offset_start as usize;
            let end = (header.offset_end as usize).min(header_bytes.len());
            if start < end {
                let raw = String::from_utf8_lossy(&header_bytes[start..end]);
                let verdicts = crate::mail::suspicion::parse_auth_results(&raw);
                msg.auth_spf = verdicts.spf;
                msg.auth_dkim = verdicts.dkim;
                msg.auth_dmarc = verdicts.dmarc;
            }
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

    // A base64 part whose closing `--boundary` never arrives (some mailers omit
    // it; a cut-short download looks the same) makes mail-parser give up on the
    // transfer encoding, and a part it could not decode is dropped from the body
    // lists and filed under attachments as raw base64 — headers with nothing
    // under them. The payload itself is intact, so close the multiparts
    // ourselves and let the parser have another go at it.
    let body = extract_body(&parsed);
    if body.html.is_some() || body.text.is_some() {
        return body;
    }
    let boundaries: Vec<String> = parsed
        .parts
        .iter()
        .filter_map(|p| p.content_type()?.attribute("boundary").map(str::to_string))
        .collect();
    if boundaries.is_empty() {
        return body;
    }
    let mut patched = raw.to_vec();
    // Innermost first: each `--b--` closes the deepest container still open.
    for b in boundaries.iter().rev() {
        patched.extend_from_slice(format!("\r\n--{b}--\r\n").as_bytes());
    }
    match MessageParser::default().parse(&patched) {
        // Whole result, not just the body: the part that was mistaken for an
        // attachment must stop being one, or its raw base64 lands on disk.
        Some(reparsed) => {
            let recovered = extract_body(&reparsed);
            if recovered.html.is_some() || recovered.text.is_some() {
                recovered
            } else {
                body
            }
        }
        None => body,
    }
}

fn extract_body(parsed: &mail_parser::Message<'_>) -> ParsedBody {
    let html = parsed.body_html(0).map(|s| s.to_string());
    // `body_text` falls back to converting the HTML part, and that conversion
    // already skips `<style>` — CSS arriving through it is CSS the message
    // deliberately shows, so only the sender's own text part is cleaned.
    let own_text_part = matches!(
        parsed.text_part(0).map(|p| &p.body),
        Some(PartType::Text(_))
    );
    let text = parsed
        .body_text(0)
        .map(|s| {
            if own_text_part {
                strip_css(&s)
            } else {
                s.into_owned()
            }
        })
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
    let out = decode_entities(&out);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The handful of entities that matter for readability. Deliberately not a full
/// entity table: this feeds text extraction, not rendering.
pub fn decode_entities(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

/// How deep an at-rule may nest before we stop looking. Mail is untrusted, and
/// a body of nothing but `{` must not cost more than one pass.
const MAX_CSS_NEST: usize = 4;

/// Element names worth recognising as selectors: mail templates open with
/// `body{margin:0}` or `td{font-size:14px}` far more often than with a class.
const HTML_TAGS: &[&str] = &[
    "a",
    "body",
    "blockquote",
    "br",
    "button",
    "center",
    "div",
    "em",
    "font",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "html",
    "img",
    "input",
    "li",
    "ol",
    "p",
    "span",
    "strong",
    "table",
    "td",
    "th",
    "tr",
    "ul",
];

/// Drop CSS rule blocks from a plain-text body.
///
/// Some senders build their `text/plain` alternative by stripping tags from
/// their own HTML — `<style>` included — so the body opens with a stylesheet.
/// The list preview then shows braces instead of words, search indexes property
/// names, and language detection sees mostly CSS. A block is removed only when
/// *both* halves look like a rule: a selector in front of the brace and a
/// `prop: value` list inside it. A message that merely talks about code, or
/// pastes JSON, keeps every character.
pub fn strip_css(text: &str) -> String {
    strip_css_at(text, 0)
}

fn strip_css_at(text: &str, depth: usize) -> String {
    if !text.contains('{') {
        return text.to_string();
    }
    let mut out = String::new();
    // Everything before `kept` is already decided; `search` never re-enters a
    // block we have looked at, so one rule is examined once.
    let mut kept = 0;
    let mut search = 0;
    let mut changed = false;
    while let Some(rel) = text[search..].find('{') {
        let open = search + rel;
        let Some(close) = matching_brace(text, open) else {
            // Unbalanced: the rest of the body stays verbatim.
            break;
        };
        // The selector is what stands between the previous line — or the
        // previous rule — and the brace. Stopping at the newline is what keeps
        // a sentence that happens to precede a rule.
        let sel_start = text[kept..open]
            .rfind(['\n', '}'])
            .map_or(kept, |i| kept + i + 1);
        if is_selector(text[sel_start..open].trim())
            && is_declarations(&text[open + 1..close], depth)
        {
            out.push_str(&text[kept..sel_start]);
            kept = close + 1;
            changed = true;
        }
        search = close + 1;
    }
    if !changed {
        return text.to_string();
    }
    out.push_str(&text[kept..]);
    out.trim().to_string()
}

/// The `}` that closes the `{` at `open`, or `None` if the braces don't balance.
fn matching_brace(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, ch) in text[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i);
                }
            }
            _ => {}
        }
    }
    None
}

fn is_selector(sel: &str) -> bool {
    if sel.is_empty() || sel.len() > 200 {
        return false;
    }
    // `@media`, `@supports`, `@font-face` — the at-rule keyword is enough,
    // because its body has to pass as rules of its own.
    if sel.starts_with('@') {
        return true;
    }
    sel.split(',').all(|part| {
        let part = part.trim();
        !part.is_empty() && part.split_whitespace().all(is_simple_selector)
    })
}

fn is_simple_selector(token: &str) -> bool {
    if !token.chars().all(|c| {
        c.is_ascii_alphanumeric()
            || matches!(
                c,
                '.' | '_'
                    | '-'
                    | '#'
                    | '*'
                    | ':'
                    | '>'
                    | '+'
                    | '~'
                    | '['
                    | ']'
                    | '('
                    | ')'
                    | '='
                    | '"'
                    | '\''
                    | '^'
                    | '$'
                    | '|'
                    | '/'
            )
    }) {
        return false;
    }
    if token.starts_with(['.', '#', '*', ':', '[', '>', '+', '~']) {
        return true;
    }
    // A bare word only counts as a selector if it names an element: this is
    // what tells `body {…}` from `struct Order {…}`.
    let name: String = token
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    HTML_TAGS.iter().any(|tag| tag.eq_ignore_ascii_case(&name))
}

fn is_declarations(inner: &str, depth: usize) -> bool {
    let inner = inner.trim();
    if inner.is_empty() {
        return false;
    }
    // An at-rule body holds rules of its own: it qualifies when stripping
    // those leaves nothing behind.
    if inner.contains('{') {
        return depth < MAX_CSS_NEST && strip_css_at(inner, depth + 1).is_empty();
    }
    let mut seen = 0;
    for decl in inner.split(';') {
        let decl = decl.trim();
        if decl.is_empty() {
            continue; // Trailing `;`.
        }
        let Some((prop, value)) = decl.split_once(':') else {
            return false;
        };
        let prop = prop.trim();
        if prop.is_empty()
            || prop.len() > 40
            || !prop.chars().all(|c| c.is_ascii_alphabetic() || c == '-')
            || value.trim().is_empty()
        {
            return false;
        }
        seen += 1;
    }
    seen > 0
}

/// The sender's own words: quoted tails, quote lines, and the signature
/// delimiter are stripped.
///
/// Only what comes *before* the attribution line survives, so a forwarded or
/// bottom-posted message reduces to nothing — callers that need text at any
/// cost (language detection) must fall back to the unstripped body.
pub fn strip_quoted(body: &str) -> String {
    let mut out = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim_start();
        // Attribution line that introduces a quoted reply.
        let attribution = (trimmed.starts_with("On ") && trimmed.ends_with("wrote:"))
            || trimmed.ends_with("пишет:")
            || trimmed.ends_with("schrieb:")
            || trimmed.ends_with("a écrit :");
        if attribution || trimmed.starts_with("-----Original Message-----") || trimmed == "-- " {
            break;
        }
        if trimmed.starts_with('>') {
            continue;
        }
        out.push(line);
    }
    out.join("\n").trim().to_string()
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
    fn strip_css_drops_a_stylesheet_flattened_into_the_text_part() {
        // The shape a real newsletter arrived in: its `text/plain` is the HTML
        // with the tags removed, so the `<style>` element came along with it.
        let body = ".bio{margin:auto}.bio .avatar{margin:auto 30px;width:fit-content}\
                    @media (max-width: 991px){.bio .avatar{margin:0 auto !important}}\
                    Hi there, here are this week's picks for you.";
        assert_eq!(
            strip_css(body),
            "Hi there, here are this week's picks for you."
        );
    }

    #[test]
    fn strip_css_removes_bare_element_rules() {
        let body = "body{margin:0;padding:0}td{font-size:14px}Welcome back.";
        assert_eq!(strip_css(body), "Welcome back.");
    }

    #[test]
    fn strip_css_keeps_a_message_that_talks_about_css() {
        // No braces at all: the fast path must hand the body back untouched.
        let body = "The margin on the banner is wrong in Outlook, and the @media \
                    query never fires. Could you check the .banner rule?";
        assert_eq!(strip_css(body), body);
    }

    #[test]
    fn strip_css_keeps_the_prose_around_an_inline_rule() {
        let body = "Please apply this:\n  .banner { margin: 0 auto; padding: 12px }\n\
                    and tell me if Outlook still breaks.";
        let out = strip_css(body);
        assert!(out.starts_with("Please apply this:"));
        assert!(out.ends_with("and tell me if Outlook still breaks."));
    }

    #[test]
    fn strip_css_leaves_code_that_is_not_css() {
        // Neither half looks like a rule: no selector, or no `prop: value`.
        for body in [
            "Payload: {\"total\": 42, \"currency\": \"EUR\"}",
            "fn main() { let x = 1; }",
            "struct Order { id: u64, total: u32 }",
        ] {
            assert_eq!(strip_css(body), body, "must not touch {body}");
        }
    }

    #[test]
    fn strip_css_survives_unbalanced_braces() {
        assert_eq!(
            strip_css("Totals {a: 1 and more text"),
            "Totals {a: 1 and more text"
        );
        let noise = "{".repeat(200);
        assert_eq!(strip_css(&noise), noise);
    }

    #[test]
    fn parse_body_cleans_a_senders_flattened_stylesheet() {
        let raw = "From: News <noreply@example.com>\r\n\
                   MIME-Version: 1.0\r\n\
                   Content-Type: multipart/alternative; boundary=\"b1\"\r\n\
                   \r\n\
                   --b1\r\n\
                   Content-Type: text/plain; charset=utf-8\r\n\
                   \r\n\
                   .bio{margin:auto}td{font-size:14px}Hi there, here are this week's picks.\r\n\
                   --b1\r\n\
                   Content-Type: text/html; charset=utf-8\r\n\
                   \r\n\
                   <html><head><style>.bio{margin:auto}</style></head>\
                   <body><p>Hi there, here are this week's picks.</p></body></html>\r\n\
                   --b1--\r\n";

        let parsed = parse_body(raw.as_bytes());
        let text = parsed.text.expect("the text part is kept");
        assert!(!text.contains('{'), "the stylesheet must be gone: {text}");
        assert!(text.starts_with("Hi there"));
        assert!(parsed.snippet.starts_with("Hi there"));
        // The HTML is what gets rendered — it must come through untouched.
        assert!(parsed.html.is_some_and(|h| h.contains("<style>")));
    }

    #[test]
    fn parse_body_keeps_css_shown_inside_html() {
        let raw = "From: Dev <dev@example.com>\r\n\
                   MIME-Version: 1.0\r\n\
                   Content-Type: text/html; charset=utf-8\r\n\
                   \r\n\
                   <html><body><p>Use this rule:</p>\
                   <pre>.banner { margin: 0 auto }</pre></body></html>\r\n";

        let parsed = parse_body(raw.as_bytes());
        let text = parsed.text.expect("text is derived from the html");
        assert!(text.contains("margin"), "quoted css must survive: {text}");
    }

    #[test]
    fn strip_quoted_keeps_only_the_words_above_the_attribution() {
        let body = "Sounds good, thanks!\n\n\
                    On Tue, Aug 4, 2026 at 10:00, Ann <ann@example.com> wrote:\n\
                    > the original question\n";
        assert_eq!(strip_quoted(body), "Sounds good, thanks!");
    }

    #[test]
    fn strip_quoted_drops_quote_lines_and_the_signature() {
        let body = "My answer\n> quoted bit\nStill mine\n-- \nSignature line";
        assert_eq!(strip_quoted(body), "My answer\nStill mine");
    }

    #[test]
    fn strip_quoted_returns_nothing_for_a_bottom_posted_reply() {
        // The whole point of the fallback in `mail::lang`: there are no own
        // words above the attribution, so detection must not rely on this.
        let body = "On Tue, Aug 4, 2026 at 10:00, Ann <ann@example.com> wrote:\n\
                    > die eigentliche Frage\n\n\
                    Meine Antwort steht darunter.";
        assert!(strip_quoted(body).is_empty());
    }

    #[test]
    fn recovers_a_base64_body_whose_closing_boundary_never_arrived() {
        // What a real sender's mailer produced: the last part's base64 ends the
        // message, with no `--b--` after it. mail-parser reads that as a broken
        // transfer encoding and files the part under attachments, undecoded —
        // headers and a blank pane. Nothing else in the payload is wrong.
        let body = base64_lines("<html><body><p>Ваш заказ подтверждён</p></body></html>");
        let raw = format!(
            "From: Travel <noreply@example.com>\r\n\
             Subject: Booking\r\n\
             MIME-Version: 1.0\r\n\
             Content-Type: multipart/alternative; boundary=\"b1\"\r\n\
             \r\n\
             --b1\r\n\
             Content-Type: text/html; charset=utf-8\r\n\
             Content-Transfer-Encoding: base64\r\n\
             \r\n\
             {body}"
        );

        let parsed = parse_body(raw.as_bytes());
        assert!(
            parsed.html.is_some_and(|h| h.contains("подтверждён")),
            "the html part must come back decoded, as the body"
        );
        assert!(
            parsed.attachments.is_empty(),
            "and must not also be filed as an attachment"
        );
        assert!(parsed.snippet.contains("подтверждён"));
    }

    #[test]
    fn recovery_closes_nested_multiparts_innermost_first() {
        let body = base64_lines("<html><body><p>Deep inside</p></body></html>");
        let raw = format!(
            "From: Travel <noreply@example.com>\r\n\
             MIME-Version: 1.0\r\n\
             Content-Type: multipart/mixed; boundary=\"outer\"\r\n\
             \r\n\
             --outer\r\n\
             Content-Type: multipart/alternative; boundary=\"inner\"\r\n\
             \r\n\
             --inner\r\n\
             Content-Type: text/html; charset=utf-8\r\n\
             Content-Transfer-Encoding: base64\r\n\
             \r\n\
             {body}"
        );

        let parsed = parse_body(raw.as_bytes());
        assert!(parsed.html.is_some_and(|h| h.contains("Deep inside")));
        assert!(parsed.attachments.is_empty());
    }

    #[test]
    fn a_well_formed_message_is_parsed_without_the_recovery_pass() {
        let body = base64_lines("<html><body><p>Nothing missing here</p></body></html>");
        let raw = format!(
            "From: Travel <noreply@example.com>\r\n\
             MIME-Version: 1.0\r\n\
             Content-Type: multipart/alternative; boundary=\"b1\"\r\n\
             \r\n\
             --b1\r\n\
             Content-Type: text/html; charset=utf-8\r\n\
             Content-Transfer-Encoding: base64\r\n\
             \r\n\
             {body}--b1--\r\n"
        );

        let parsed = parse_body(raw.as_bytes());
        assert!(parsed.html.is_some_and(|h| h.contains("Nothing missing")));
        assert!(parsed.attachments.is_empty());
    }

    /// Base64 the way mailers write it: 76-char lines, each CRLF-terminated.
    fn base64_lines(content: &str) -> String {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(content);
        encoded
            .as_bytes()
            .chunks(76)
            .map(|c| format!("{}\r\n", std::str::from_utf8(c).unwrap()))
            .collect()
    }

    #[test]
    fn decode_entities_covers_the_readability_set() {
        assert_eq!(
            decode_entities("a&nbsp;b &amp; &lt;c&gt; &quot;d&quot; &#39;e&#39;"),
            "a b & <c> \"d\" 'e'"
        );
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
    fn captures_auth_results_and_reply_to() {
        let msg = headers(
            "From: PayPal <service@paypal.com>\r\n\
             Reply-To: Support <help@evil.example>\r\n\
             Authentication-Results: mx.google.com;\r\n\
             \tspf=pass smtp.mailfrom=paypal.com;\r\n\
             \tdkim=pass header.d=paypal.com;\r\n\
             \tdmarc=fail (p=REJECT) header.from=paypal.com\r\n\
             Subject: Hi\r\n\
             \r\n",
        );
        assert_eq!(msg.reply_to_addr.as_deref(), Some("help@evil.example"));
        assert_eq!(msg.auth_spf.as_deref(), Some("pass"));
        assert_eq!(msg.auth_dkim.as_deref(), Some("pass"));
        assert_eq!(msg.auth_dmarc.as_deref(), Some("fail"));
    }

    #[test]
    fn first_auth_results_header_wins() {
        let msg = headers(
            "Authentication-Results: mx.example.com; dmarc=pass\r\n\
             Authentication-Results: forwarder.example; dmarc=fail\r\n\
             From: A <a@example.com>\r\n\
             \r\n",
        );
        assert_eq!(msg.auth_dmarc.as_deref(), Some("pass"));
    }

    #[test]
    fn absent_security_headers_stay_none() {
        let msg = headers("From: A Friend <friend@example.com>\r\nSubject: Hi\r\n\r\n");
        assert!(msg.reply_to_addr.is_none());
        assert!(msg.auth_spf.is_none());
        assert!(msg.auth_dkim.is_none());
        assert!(msg.auth_dmarc.is_none());
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
}
