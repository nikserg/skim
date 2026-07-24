//! Local phishing heuristics: per-link checks over sanitized HTML and
//! sender-level checks over stored headers. Everything runs offline and
//! deterministically — no reputation services, no thresholds to tune.
//!
//! Scoring is two-tier by design. *Hard* link signals (display/target
//! mismatch, `user@host` tricks, raw-IP hosts) always flag. *Soft* signals
//! (shorteners, plain http, punycode, odd ports, deep subdomains) flag only
//! when the sender itself already looks suspicious — otherwise every
//! newsletter full of tracking redirects would cry wolf and train the user
//! to ignore warnings.

use crate::db::models::{LinkFlag, SecuritySignals, SignalReason};
use rusqlite::{Connection, OptionalExtension};
use url::{Host, Url};

/// Verdicts pulled out of an `Authentication-Results` header.
#[derive(Debug, Default, PartialEq)]
pub struct AuthVerdicts {
    pub spf: Option<String>,
    pub dkim: Option<String>,
    pub dmarc: Option<String>,
}

/// Extract `spf=`/`dkim=`/`dmarc=` verdicts from a raw `Authentication-Results`
/// value. Tolerates folding whitespace, comments in parentheses, and property
/// tails (`smtp.mailfrom=…`); the first verdict per method wins.
pub fn parse_auth_results(raw: &str) -> AuthVerdicts {
    let mut out = AuthVerdicts::default();
    // Strip CFWS comments so "(p=REJECT)" can't be mistaken for a token.
    let mut cleaned = String::with_capacity(raw.len());
    let mut depth = 0usize;
    for ch in raw.chars() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 => cleaned.push(ch),
            _ => {}
        }
    }
    for token in cleaned.split([';', ' ', '\t', '\r', '\n']) {
        let Some((method, value)) = token.trim().split_once('=') else {
            continue;
        };
        let method = method.trim().to_ascii_lowercase();
        let value: String = value
            .trim()
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        if value.is_empty() {
            continue;
        }
        let slot = match method.as_str() {
            "spf" => &mut out.spf,
            "dkim" => &mut out.dkim,
            "dmarc" => &mut out.dmarc,
            _ => continue,
        };
        if slot.is_none() {
            *slot = Some(value);
        }
    }
    out
}

/// Two-part public suffixes we special-case so `example.co.uk` registers as
/// `example.co.uk`, not `co.uk`. A deliberate mini-list, not the PSL — the
/// distance-1 and frequency gates around every comparison absorb the rare
/// miss, and a full PSL is a dependency this heuristic doesn't earn.
const TWO_PART_SUFFIXES: &[&str] = &[
    "co.uk", "org.uk", "ac.uk", "gov.uk", "co.jp", "ne.jp", "or.jp", "com.au", "net.au", "org.au",
    "com.br", "com.mx", "com.ar", "co.in", "co.nz", "co.za", "com.tr", "com.cn",
];

/// Last-two-labels registrable domain, extended by the suffix mini-list.
/// Input must already be lowercase (hosts from the `url` crate are).
pub fn registrable_domain(host: &str) -> &str {
    let host = host.trim_end_matches('.');
    let mut parts = host.rsplit('.');
    let (Some(tld), Some(sld)) = (parts.next(), parts.next()) else {
        return host;
    };
    let take = if TWO_PART_SUFFIXES.contains(&&host[host.len() - tld.len() - sld.len() - 1..])
        && parts.clone().next().is_some()
    {
        3
    } else {
        2
    };
    let labels: Vec<&str> = host.rsplit('.').take(take).collect();
    let len: usize = labels.iter().map(|l| l.len() + 1).sum::<usize>() - 1;
    &host[host.len() - len..]
}

/// Damerau-Levenshtein distance == 1 (one edit or one transposition).
/// Distance 0 (equality) deliberately returns false — the lookalike check
/// wants "almost but not quite the same".
pub fn damerau1(a: &str, b: &str) -> bool {
    if a == b {
        return false;
    }
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (la, lb) = (a.len(), b.len());
    match la.abs_diff(lb) {
        0 => {
            // Same length: exactly one substitution, or one adjacent swap.
            let diffs: Vec<usize> = (0..la).filter(|&i| a[i] != b[i]).collect();
            match diffs.len() {
                1 => true,
                2 => {
                    let (i, j) = (diffs[0], diffs[1]);
                    j == i + 1 && a[i] == b[j] && a[j] == b[i]
                }
                _ => false,
            }
        }
        1 => {
            // One insertion/deletion: skip the first mismatch in the longer.
            let (long, short) = if la > lb { (&a, &b) } else { (&b, &a) };
            let mut i = 0;
            while i < short.len() && long[i] == short[i] {
                i += 1;
            }
            long[i + 1..] == short[i..]
        }
        _ => false,
    }
}

/// Pull `(href, text)` pairs out of sanitizer output. Both ammonia and our
/// own `linkify` emit lowercase `<a>` tags with double-quoted attributes and
/// entity-escaped values, so a scanner in the spirit of `html_to_text` is
/// enough — no HTML parser needed. Values are entity-decoded to match what
/// `getAttribute("href")` returns in the frontend.
pub fn extract_links(html: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(open) = find_ignore_ascii_case(rest, "<a") {
        let after_open = &rest[open + 2..];
        // Require a real tag boundary so "<abbr>" doesn't match.
        if !after_open.starts_with([' ', '\t', '\n', '\r', '>']) {
            rest = after_open;
            continue;
        }
        let Some(tag_end) = after_open.find('>') else {
            break;
        };
        let tag = &after_open[..tag_end];
        let body_start = &after_open[tag_end + 1..];
        let close = find_ignore_ascii_case(body_start, "</a").unwrap_or(body_start.len());
        let inner = &body_start[..close];
        if let Some(href) = attr_value(tag, "href") {
            let text = super::parse::html_to_text(inner);
            out.push((decode_entities(&href), text));
        }
        rest = &body_start[close..];
        if rest.is_empty() {
            break;
        }
        rest = &rest[1..];
    }
    out
}

fn find_ignore_ascii_case(haystack: &str, needle: &str) -> Option<usize> {
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    h.windows(n.len().min(h.len()).max(1))
        .position(|w| w.eq_ignore_ascii_case(n))
        .filter(|_| h.len() >= n.len())
}

/// Read a double-quoted attribute out of a tag's attribute string. Quotes
/// inside values are entity-escaped by the serializer, so the first closing
/// quote is always the real one.
fn attr_value(tag: &str, name: &str) -> Option<String> {
    let pat = format!("{name}=\"");
    let start = find_ignore_ascii_case(tag, &pat)? + pat.len();
    let end = tag[start..].find('"')?;
    Some(tag[start..start + end].to_string())
}

/// Decode the entities the sanitizer's serializer can emit in attributes and
/// text, plus numeric forms for safety.
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        let tail = &rest[pos..];
        let Some(semi) = tail.find(';').filter(|&i| i <= 12) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let entity = &tail[1..semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => entity
                .strip_prefix('#')
                .and_then(|num| {
                    num.strip_prefix(['x', 'X']).map_or_else(
                        || num.parse::<u32>().ok(),
                        |h| u32::from_str_radix(h, 16).ok(),
                    )
                })
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &tail[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Does the anchor's *visible text*, as a whole, look like a URL or bare
/// domain? This is the gate for the display/target mismatch check: "Read
/// more" pointing at a tracking redirect is normal mail, "paypal.com"
/// pointing elsewhere is a phish.
pub fn looks_like_url(text: &str) -> bool {
    let t = text.trim().trim_end_matches(['.', ',', ';', ')', ']']);
    if t.is_empty() || t.contains(char::is_whitespace) {
        return false;
    }
    let lower = t.to_ascii_lowercase();
    if let Some(rest) = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
    {
        return !rest.is_empty();
    }
    let host = lower
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("");
    domain_shape(host)
}

/// Bare-domain shape check for anchor text. The extension list keeps file
/// names that happen to end in a ccTLD ("main.rs", "index.js" in GitHub
/// notification mail) from reading as domains — a missed warning is cheaper
/// than teaching the user to ignore warnings.
fn domain_shape(host: &str) -> bool {
    const FILE_EXTENSIONS: &[&str] = &[
        "js", "jsx", "ts", "tsx", "css", "json", "md", "rs", "py", "go", "sh", "rb", "yml", "yaml",
        "toml", "txt", "log", "lock", "java", "kt", "php",
    ];
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 {
        return false;
    }
    let tld = labels[labels.len() - 1];
    if tld.len() < 2 || !tld.chars().all(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    if labels.len() == 2 && FILE_EXTENSIONS.contains(&tld) {
        return false;
    }
    labels.iter().all(|l| {
        !l.is_empty()
            && l.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || !c.is_ascii())
    })
}

const SHORTENERS: &[&str] = &[
    "bit.ly",
    "tinyurl.com",
    "t.co",
    "goo.gl",
    "ow.ly",
    "is.gd",
    "buff.ly",
    "cutt.ly",
    "rb.gy",
    "tiny.cc",
    "s.id",
    "lnkd.in",
    "rebrand.ly",
];

fn reason(code: &str, param: Option<String>) -> SignalReason {
    SignalReason {
        code: code.to_string(),
        param,
    }
}

/// Analyze one `(href, text)` pair. Returns `None` for clean links and for
/// non-http(s) schemes (mailto needs no gate).
pub fn analyze_link(href: &str, text: &str, sender_suspicious: bool) -> Option<LinkFlag> {
    let url = Url::parse(href).ok()?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return None;
    }
    let host = url.host_str()?.to_string();
    let mut reasons = Vec::new();

    // Hard signals — always flagged.
    if !url.username().is_empty() || url.password().is_some() {
        reasons.push(reason("userinfo", None));
    }
    if matches!(url.host(), Some(Host::Ipv4(_) | Host::Ipv6(_))) {
        reasons.push(reason("ip", None));
    }
    if looks_like_url(text) {
        // Parse the text as a URL too, so IDNA normalizes both sides before
        // comparing registrable domains.
        let t = text.trim().trim_end_matches(['.', ',', ';', ')', ']']);
        let candidate = if t.to_ascii_lowercase().starts_with("http") {
            t.to_string()
        } else {
            format!("https://{t}")
        };
        if let Some(text_host) = Url::parse(&candidate)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
        {
            if !matches!(url.host(), Some(Host::Ipv4(_) | Host::Ipv6(_)))
                && registrable_domain(&text_host) != registrable_domain(&host)
            {
                reasons.push(reason(
                    "mismatch",
                    Some(registrable_domain(&text_host).to_string()),
                ));
            }
        }
    }

    // Soft signals — only worth surfacing when the sender already smells off.
    if sender_suspicious {
        if SHORTENERS.contains(&registrable_domain(&host)) {
            reasons.push(reason("shortener", None));
        }
        if url.scheme() == "http" {
            reasons.push(reason("http", None));
        }
        if host.split('.').any(|l| l.starts_with("xn--")) {
            reasons.push(reason("punycode", None));
        }
        if url.port().is_some() {
            reasons.push(reason("port", None));
        }
        if host.split('.').count() > 5 {
            reasons.push(reason("subdomains", None));
        }
    }

    if reasons.is_empty() {
        return None;
    }
    Some(LinkFlag {
        href: href.to_string(),
        host,
        reasons,
    })
}

struct MessageRow {
    from_name: Option<String>,
    from_addr: Option<String>,
    reply_to_addr: Option<String>,
    auth_spf: Option<String>,
    auth_dkim: Option<String>,
    auth_dmarc: Option<String>,
}

fn load_row(conn: &Connection, message_id: i64) -> rusqlite::Result<Option<MessageRow>> {
    conn.query_row(
        "SELECT from_name, from_addr, reply_to_addr, auth_spf, auth_dkim, auth_dmarc
         FROM messages WHERE id = ?1",
        [message_id],
        |r| {
            Ok(MessageRow {
                from_name: r.get(0)?,
                from_addr: r.get(1)?,
                reply_to_addr: r.get(2)?,
                auth_spf: r.get(3)?,
                auth_dkim: r.get(4)?,
                auth_dmarc: r.get(5)?,
            })
        },
    )
    .optional()
}

fn addr_domain(addr: &str) -> Option<String> {
    let domain = addr.rsplit('@').next()?.trim().to_ascii_lowercase();
    if domain.is_empty() || domain == addr.to_ascii_lowercase() || !domain.contains('.') {
        return None;
    }
    Some(domain)
}

/// First email-looking token embedded in a display name, if any.
fn embedded_email(name: &str) -> Option<String> {
    name.split([' ', '\t', '<', '>', '(', ')', '"', '\'', ',', ';'])
        .map(|t| t.trim_matches(['.', ':', '[', ']']))
        .find(|t| {
            t.split_once('@')
                .is_some_and(|(l, d)| !l.is_empty() && d.contains('.'))
        })
        .map(str::to_string)
}

/// How many messages from this exact address exist (including the one being
/// looked at), and how many from its whole domain.
fn sender_history(
    conn: &Connection,
    from_addr: &str,
    from_domain: &str,
) -> rusqlite::Result<(i64, i64)> {
    let addr_count: i64 = conn.query_row(
        "SELECT count(*) FROM messages WHERE lower(from_addr) = lower(?1)",
        [from_addr],
        |r| r.get(0),
    )?;
    let domain_count: i64 = conn.query_row(
        "SELECT count(*) FROM messages WHERE lower(from_addr) LIKE '%@' || ?1",
        [from_domain],
        |r| r.get(0),
    )?;
    Ok((addr_count, domain_count))
}

/// Message-level suspicion signals. Own/outgoing mail (From = one of the
/// user's accounts) never flags. Missing data (no auth headers, unknown
/// sender history) is never treated as suspicious by itself.
pub fn sender_signals(conn: &Connection, message_id: i64) -> rusqlite::Result<Vec<SignalReason>> {
    let Some(row) = load_row(conn, message_id)? else {
        return Ok(vec![]);
    };
    let Some(from_addr) = row.from_addr.as_deref() else {
        return Ok(vec![]);
    };
    let own: Vec<String> = conn
        .prepare("SELECT lower(email) FROM accounts")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    if own.contains(&from_addr.to_ascii_lowercase()) {
        return Ok(vec![]);
    }
    let Some(from_domain) = addr_domain(from_addr) else {
        return Ok(vec![]);
    };

    let mut out = Vec::new();

    if row.auth_dmarc.as_deref() == Some("fail") {
        out.push(reason("auth_dmarc_fail", None));
    } else if row.auth_spf.as_deref() == Some("fail") && row.auth_dkim.as_deref() == Some("fail") {
        out.push(reason("auth_fail", None));
    }

    if let Some(embedded) = row.from_name.as_deref().and_then(embedded_email) {
        if let Some(embedded_domain) = addr_domain(&embedded) {
            if registrable_domain(&embedded_domain) != registrable_domain(&from_domain) {
                out.push(reason("name_addr_mismatch", Some(embedded)));
            }
        }
    }

    let (addr_count, domain_count) = sender_history(conn, from_addr, &from_domain)?;
    let first_time = addr_count <= 1;

    if first_time {
        if let Some(reply_domain) = row.reply_to_addr.as_deref().and_then(addr_domain) {
            if registrable_domain(&reply_domain) != registrable_domain(&from_domain) {
                out.push(reason("reply_to_mismatch", Some(reply_domain)));
            }
        }
    }

    // Lookalike: this domain is nearly unseen, but one edit away from a
    // domain the user hears from regularly, on the same public suffix.
    if domain_count < 2 {
        let from_reg = registrable_domain(&from_domain).to_string();
        let (from_label, from_suffix) = from_reg.split_once('.').unwrap_or((from_reg.as_str(), ""));
        let mut stmt = conn.prepare(
            "SELECT lower(substr(from_addr, instr(from_addr, '@') + 1)) AS d, count(*) AS c
             FROM messages WHERE from_addr LIKE '%@%.%' GROUP BY d HAVING c >= 3",
        )?;
        let frequent: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<_>>()?;
        for freq in frequent {
            let freq_reg = registrable_domain(&freq);
            if freq_reg == from_reg {
                continue;
            }
            let (freq_label, freq_suffix) = freq_reg.split_once('.').unwrap_or((freq_reg, ""));
            if freq_suffix == from_suffix && damerau1(from_label, freq_label) {
                out.push(reason("lookalike_domain", Some(freq_reg.to_string())));
                break;
            }
        }
    }

    Ok(out)
}

/// Full verdict for a rendered message: sender signals plus per-link flags
/// over the sanitized HTML. `None` when nothing fired at all.
pub fn security_for_message(
    conn: &Connection,
    message_id: i64,
    sanitized_html: &str,
) -> rusqlite::Result<Option<SecuritySignals>> {
    let sender = sender_signals(conn, message_id)?;
    let sender_suspicious = !sender.is_empty();
    let links: Vec<LinkFlag> = extract_links(sanitized_html)
        .iter()
        .filter_map(|(href, text)| analyze_link(href, text, sender_suspicious))
        .collect();
    if sender.is_empty() && links.is_empty() {
        return Ok(None);
    }
    Ok(Some(SecuritySignals { sender, links }))
}

/// Compact plain-text report for the AI prompt: fired signals, raw auth
/// verdicts, sender history, and a link inventory. `None` when the message
/// is clean — the AI then gets no security block at all.
pub fn prompt_summary(
    conn: &Connection,
    message_id: i64,
    sanitized_html: &str,
) -> rusqlite::Result<Option<String>> {
    let Some(signals) = security_for_message(conn, message_id, sanitized_html)? else {
        return Ok(None);
    };
    let row = load_row(conn, message_id)?;
    let mut out = String::new();
    for s in &signals.sender {
        out.push_str("- signal: ");
        out.push_str(&s.code);
        if let Some(p) = &s.param {
            out.push_str(" (");
            out.push_str(p);
            out.push(')');
        }
        out.push('\n');
    }
    if let Some(row) = row {
        let fmt = |v: &Option<String>| v.clone().unwrap_or_else(|| "not present".into());
        out.push_str(&format!(
            "- authentication: spf={}, dkim={}, dmarc={}\n",
            fmt(&row.auth_spf),
            fmt(&row.auth_dkim),
            fmt(&row.auth_dmarc)
        ));
        if let Some(reply_to) = &row.reply_to_addr {
            out.push_str(&format!("- reply-to: {reply_to}\n"));
        }
        if let (Some(addr), Some(domain)) = (
            row.from_addr.as_deref(),
            row.from_addr.as_deref().and_then(addr_domain),
        ) {
            let (addr_count, _) = sender_history(conn, addr, &domain)?;
            if addr_count <= 1 {
                out.push_str("- first message ever received from this sender\n");
            }
        }
    }
    for flag in &signals.links {
        let codes: Vec<&str> = flag.reasons.iter().map(|r| r.code.as_str()).collect();
        out.push_str(&format!(
            "- flagged link: {} [{}]\n",
            flag.href,
            codes.join(", ")
        ));
    }
    let links = extract_links(sanitized_html);
    if !links.is_empty() {
        out.push_str("- all links in the message:\n");
        for (href, _) in links.iter().take(15) {
            out.push_str(&format!("  {href}\n"));
        }
        if links.len() > 15 {
            out.push_str(&format!("  … and {} more\n", links.len() - 15));
        }
    }
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    // ---- parse_auth_results ----

    #[test]
    fn auth_results_gmail_style() {
        let v = parse_auth_results(
            "mx.google.com;\r\n\tspf=pass (google.com: domain of x designates y) \
             smtp.mailfrom=paypal.com;\r\n\tdkim=pass header.d=paypal.com;\r\n\t\
             dmarc=fail (p=REJECT sp=REJECT) header.from=paypal.com",
        );
        assert_eq!(v.spf.as_deref(), Some("pass"));
        assert_eq!(v.dkim.as_deref(), Some("pass"));
        assert_eq!(v.dmarc.as_deref(), Some("fail"));
    }

    #[test]
    fn auth_results_comment_cannot_forge_verdict() {
        // "dmarc=pass" only inside a comment must not count.
        let v = parse_auth_results("mx.example.com; (dmarc=pass) dmarc=fail");
        assert_eq!(v.dmarc.as_deref(), Some("fail"));
    }

    #[test]
    fn auth_results_first_verdict_per_method_wins() {
        let v = parse_auth_results("h; dkim=pass header.d=a.com; dkim=fail header.d=b.com");
        assert_eq!(v.dkim.as_deref(), Some("pass"));
    }

    // ---- registrable_domain ----

    #[test]
    fn registrable_domain_cases() {
        assert_eq!(registrable_domain("www.paypal.com"), "paypal.com");
        assert_eq!(registrable_domain("paypal.com"), "paypal.com");
        assert_eq!(registrable_domain("a.b.example.co.uk"), "example.co.uk");
        assert_eq!(registrable_domain("co.uk"), "co.uk");
        assert_eq!(registrable_domain("localhost"), "localhost");
    }

    // ---- damerau1 ----

    #[test]
    fn damerau1_cases() {
        assert!(damerau1("paypal", "paypa1")); // substitution
        assert!(damerau1("paypal", "payapl")); // transposition
        assert!(damerau1("paypal", "papal")); // deletion
        assert!(damerau1("paypal", "paypall")); // insertion
        assert!(!damerau1("paypal", "paypal")); // equal → not a lookalike
        assert!(!damerau1("paypal", "amazon"));
        assert!(!damerau1("paypal", "pyapla"));
    }

    // ---- extract_links (against real sanitizer output) ----

    #[test]
    fn extract_links_from_sanitizer_output() {
        let s = super::super::sanitize::sanitize_email_html(
            r#"<p>Hi <a href="https://example.com/a?x=1&y=2" title="t">click <b>here</b></a>
               and <a href="mailto:a@b.com">mail</a></p>"#,
            1,
            true,
        );
        let links = extract_links(&s.html);
        assert_eq!(links.len(), 2);
        // Entity-decoded, exactly what getAttribute("href") will return.
        assert_eq!(links[0].0, "https://example.com/a?x=1&y=2");
        assert_eq!(links[0].1, "click here");
        assert_eq!(links[1].0, "mailto:a@b.com");
    }

    #[test]
    fn extract_links_from_linkified_plaintext() {
        let html = super::super::sanitize::text_to_html("see https://example.com/x. bye");
        let links = extract_links(&html);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].0, "https://example.com/x");
        assert_eq!(links[0].1, "https://example.com/x");
    }

    // ---- looks_like_url ----

    #[test]
    fn looks_like_url_cases() {
        assert!(looks_like_url("https://paypal.com/login"));
        assert!(looks_like_url("paypal.com"));
        assert!(looks_like_url("www.paypal.com/security."));
        assert!(!looks_like_url("Read the full story"));
        assert!(!looks_like_url("Click here"));
        assert!(!looks_like_url("Visit paypal.com today")); // whole text isn't a URL
        assert!(!looks_like_url("main.rs")); // filename, not a domain
        assert!(!looks_like_url("index.js"));
        assert!(!looks_like_url(""));
    }

    // ---- analyze_link ----

    #[test]
    fn newsletter_tracking_redirect_is_clean() {
        // THE key regression: ordinary "click here" text over a tracker
        // domain, clean sender → no flag at all.
        assert!(analyze_link(
            "https://click.mailchimp.example/track?u=1",
            "Read the full story",
            false,
        )
        .is_none());
    }

    #[test]
    fn soft_signals_stay_dormant_for_clean_sender() {
        assert!(analyze_link("https://bit.ly/x", "Click here", false).is_none());
        assert!(analyze_link("http://example.com/", "here", false).is_none());
        let flagged = analyze_link("https://bit.ly/x", "Click here", true).unwrap();
        assert_eq!(flagged.reasons[0].code, "shortener");
    }

    #[test]
    fn display_target_mismatch_fires() {
        let flag = analyze_link(
            "https://evil.example/verify",
            "https://account.microsoft.com/security",
            false,
        )
        .unwrap();
        assert_eq!(flag.reasons[0].code, "mismatch");
        assert_eq!(flag.reasons[0].param.as_deref(), Some("microsoft.com"));
    }

    #[test]
    fn same_domain_text_is_clean() {
        assert!(analyze_link("https://www.example.com/deep/path", "example.com", false).is_none());
    }

    #[test]
    fn linkified_plaintext_cannot_mismatch() {
        // text == href by construction.
        assert!(analyze_link("https://example.com/x", "https://example.com/x", false).is_none());
    }

    #[test]
    fn userinfo_trick_fires() {
        let flag = analyze_link("https://paypal.com@evil.example/", "log in", false).unwrap();
        assert!(flag.reasons.iter().any(|r| r.code == "userinfo"));
    }

    #[test]
    fn ip_host_fires_without_bogus_mismatch() {
        let flag = analyze_link("https://185.199.1.1/verify", "click", false).unwrap();
        assert_eq!(flag.reasons.len(), 1);
        assert_eq!(flag.reasons[0].code, "ip");
    }

    #[test]
    fn punycode_lookalike_text_mismatches() {
        // Displayed "paypal.com", actually xn--pypal-4ve.com (pаypal with a
        // Cyrillic а). IDNA keeps them distinct registrable domains.
        let flag = analyze_link("https://xn--pypal-4ve.com/login", "paypal.com", false).unwrap();
        assert!(flag.reasons.iter().any(|r| r.code == "mismatch"));
    }

    #[test]
    fn mailto_is_ignored() {
        assert!(analyze_link("mailto:a@b.com", "a@b.com", true).is_none());
    }

    // ---- sender signals over a real (in-memory) DB ----

    fn test_db() -> Db {
        let db = Db::open_in_memory().unwrap();
        db.with(|conn| {
            conn.execute_batch(
                "INSERT INTO accounts (id, email, provider, imap_host, smtp_host, created_at)
                 VALUES ('acct', 'me@mymail.com', 'imap', 'i', 's', 0);
                 INSERT INTO folders (id, account_id, imap_name, display_name)
                 VALUES (1, 'acct', 'INBOX', 'Inbox');",
            )
        })
        .unwrap();
        db
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_message(
        db: &Db,
        id: i64,
        from_name: Option<&str>,
        from_addr: &str,
        reply_to: Option<&str>,
        dmarc: Option<&str>,
        spf: Option<&str>,
        dkim: Option<&str>,
    ) {
        let (from_name, from_addr, reply_to, dmarc, spf, dkim) = (
            from_name.map(str::to_string),
            from_addr.to_string(),
            reply_to.map(str::to_string),
            dmarc.map(str::to_string),
            spf.map(str::to_string),
            dkim.map(str::to_string),
        );
        db.with(move |conn| {
            conn.execute(
                "INSERT INTO messages (id, account_id, folder_id, uid, date, from_name, from_addr,
                                       reply_to_addr, auth_dmarc, auth_spf, auth_dkim)
                 VALUES (?1, 'acct', 1, ?1, 0, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![id, from_name, from_addr, reply_to, dmarc, spf, dkim],
            )
        })
        .unwrap();
    }

    #[test]
    fn dmarc_fail_alone_fires() {
        let db = test_db();
        insert_message(
            &db,
            10,
            None,
            "x@shop.example",
            None,
            Some("fail"),
            None,
            None,
        );
        db.with(|conn| {
            let s = sender_signals(conn, 10)?;
            assert_eq!(s.len(), 1);
            assert_eq!(s[0].code, "auth_dmarc_fail");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn clean_first_time_sender_is_not_suspicious() {
        let db = test_db();
        insert_message(
            &db,
            10,
            Some("A Friend"),
            "friend@example.com",
            None,
            Some("pass"),
            Some("pass"),
            Some("pass"),
        );
        db.with(|conn| {
            assert!(sender_signals(conn, 10)?.is_empty());
            assert!(security_for_message(conn, 10, "<p>hi</p>")?.is_none());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn missing_auth_headers_are_not_suspicious() {
        let db = test_db();
        insert_message(&db, 10, None, "a@example.com", None, None, None, None);
        db.with(|conn| {
            assert!(sender_signals(conn, 10)?.is_empty());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn own_outgoing_mail_never_flags() {
        let db = test_db();
        insert_message(
            &db,
            10,
            None,
            "me@mymail.com",
            Some("other@x.com"),
            Some("fail"),
            None,
            None,
        );
        db.with(|conn| {
            assert!(sender_signals(conn, 10)?.is_empty());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn display_name_with_foreign_email_fires() {
        let db = test_db();
        insert_message(
            &db,
            10,
            Some("service@paypal.com via notify"),
            "x@evil.example",
            None,
            None,
            None,
            None,
        );
        db.with(|conn| {
            let s = sender_signals(conn, 10)?;
            assert_eq!(s[0].code, "name_addr_mismatch");
            assert_eq!(s[0].param.as_deref(), Some("service@paypal.com"));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn reply_to_mismatch_gated_by_first_contact() {
        let db = test_db();
        // First contact + foreign Reply-To → fires.
        insert_message(
            &db,
            10,
            None,
            "a@shop.example",
            Some("collect@evil.example"),
            None,
            None,
            None,
        );
        db.with(|conn| {
            let s = sender_signals(conn, 10)?;
            assert_eq!(s.len(), 1);
            assert_eq!(s[0].code, "reply_to_mismatch");
            Ok(())
        })
        .unwrap();
        // Known sender (2nd message) + foreign Reply-To → does not fire.
        insert_message(
            &db,
            11,
            None,
            "a@shop.example",
            Some("collect@evil.example"),
            None,
            None,
            None,
        );
        db.with(|conn| {
            assert!(sender_signals(conn, 11)?.is_empty());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn lookalike_domain_fires_against_frequent_correspondent() {
        let db = test_db();
        for i in 0..5 {
            insert_message(
                &db,
                100 + i,
                None,
                "service@paypal.com",
                None,
                None,
                None,
                None,
            );
        }
        insert_message(&db, 200, None, "service@paypa1.com", None, None, None, None);
        db.with(|conn| {
            let s = sender_signals(conn, 200)?;
            assert_eq!(s.len(), 1);
            assert_eq!(s[0].code, "lookalike_domain");
            assert_eq!(s[0].param.as_deref(), Some("paypal.com"));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn frequent_domain_itself_is_not_a_lookalike() {
        let db = test_db();
        for i in 0..5 {
            insert_message(
                &db,
                100 + i,
                None,
                "service@paypal.com",
                None,
                None,
                None,
                None,
            );
        }
        insert_message(&db, 200, None, "billing@paypal.com", None, None, None, None);
        db.with(|conn| {
            assert!(sender_signals(conn, 200)?.is_empty());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn different_suffix_same_brand_is_not_a_lookalike() {
        // amazon.de → amazon.com must not flag (same brand, other country).
        let db = test_db();
        for i in 0..5 {
            insert_message(&db, 100 + i, None, "info@amazon.de", None, None, None, None);
        }
        insert_message(&db, 200, None, "info@amazon.com", None, None, None, None);
        db.with(|conn| {
            assert!(sender_signals(conn, 200)?.is_empty());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn security_for_message_combines_and_gates_soft_signals() {
        let db = test_db();
        insert_message(
            &db,
            10,
            None,
            "x@shop.example",
            None,
            Some("fail"),
            None,
            None,
        );
        db.with(|conn| {
            let html = super::super::sanitize::sanitize_email_html(
                r#"<a href="https://bit.ly/x">Click here</a>"#,
                10,
                true,
            )
            .html;
            // Suspicious sender → the shortener soft signal surfaces.
            let sig = security_for_message(conn, 10, &html)?.expect("signals");
            assert_eq!(sig.sender[0].code, "auth_dmarc_fail");
            assert_eq!(sig.links.len(), 1);
            assert_eq!(sig.links[0].reasons[0].code, "shortener");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn prompt_summary_lists_signals_and_links() {
        let db = test_db();
        insert_message(
            &db,
            10,
            None,
            "x@shop.example",
            None,
            Some("fail"),
            None,
            None,
        );
        db.with(|conn| {
            let html =
                r#"<a href="https://evil.example/v" rel="noopener noreferrer">paypal.com</a>"#;
            let summary = prompt_summary(conn, 10, html)?.expect("summary");
            assert!(summary.contains("auth_dmarc_fail"));
            assert!(summary.contains("flagged link: https://evil.example/v"));
            assert!(summary.contains("first message ever received"));
            Ok(())
        })
        .unwrap();
    }
}
