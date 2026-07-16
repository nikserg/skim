//! Outgoing mail: MIME construction and SMTP submission via lettre.

use crate::db::drafts::Draft;
use crate::db::models::Account;
use crate::error::{Result, SkimError};
use crate::mail::imap_client::Credentials;
use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, Message, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::{Credentials as SmtpCredentials, Mechanism};
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

/// Parse a raw comma/semicolon-separated recipient string.
pub fn parse_recipients(raw: &str) -> Vec<String> {
    raw.split([',', ';'])
        .map(|s| s.trim())
        .filter(|s| s.contains('@'))
        .map(|s| s.to_string())
        .collect()
}

pub struct OutgoingRefs {
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
}

/// Build the RFC822 message for a draft. Returns the raw bytes so the same
/// payload can be submitted over SMTP and appended to the Sent folder.
///
/// `attachments` is a list of `(filename, mime_type, bytes)`. With none, the
/// message is a single text/plain part (unchanged from plain mail); with any,
/// it becomes a `multipart/mixed` of the body plus one part per file.
///
/// `message_id` overrides the RFC822 Message-ID (given without angle brackets);
/// pass `Some` to keep a stable identity when the same draft is saved back to
/// the IMAP Drafts folder and later sent, `None` to let lettre generate one.
pub fn build_message(
    account: &Account,
    draft: &Draft,
    refs: &OutgoingRefs,
    attachments: &[(String, String, Vec<u8>)],
    message_id: Option<&str>,
) -> Result<Vec<u8>> {
    let from: Mailbox = match &account.display_name {
        Some(name) if !name.is_empty() => format!("{} <{}>", name, account.email),
        _ => account.email.clone(),
    }
    .parse()
    .map_err(|e| SkimError::other("send", format!("invalid sender: {e}")))?;

    let mut builder = Message::builder().from(from);

    let to = parse_recipients(&draft.to);
    if to.is_empty() {
        return Err(SkimError::other("send", "no valid recipients"));
    }
    for addr in &to {
        builder = builder.to(addr
            .parse()
            .map_err(|e| SkimError::other("send", format!("invalid recipient {addr}: {e}")))?);
    }
    for addr in parse_recipients(&draft.cc) {
        builder = builder.cc(addr
            .parse()
            .map_err(|e| SkimError::other("send", format!("invalid recipient {addr}: {e}")))?);
    }
    for addr in parse_recipients(&draft.bcc) {
        builder = builder.bcc(
            addr.parse()
                .map_err(|e| SkimError::other("send", format!("invalid recipient {addr}: {e}")))?,
        );
    }

    builder = builder.subject(&draft.subject);

    if let Some(mid) = message_id {
        // lettre uses the given value verbatim (unlike its auto-generated id,
        // which it wraps); our ids are stored bare, so add the angle brackets.
        let bare = mid.trim_matches(|c| c == '<' || c == '>');
        builder = builder.message_id(Some(format!("<{bare}>")));
    }

    if let Some(irt) = &refs.in_reply_to {
        builder = builder.in_reply_to(irt.clone());
    }
    if !refs.references.is_empty() {
        builder = builder.references(refs.references.join(" "));
    }

    let message = if attachments.is_empty() {
        builder.body(draft.body.clone())
    } else {
        let mut multipart = MultiPart::mixed().singlepart(SinglePart::plain(draft.body.clone()));
        for (filename, mime_type, bytes) in attachments {
            let content_type = ContentType::parse(mime_type)
                .unwrap_or(ContentType::parse("application/octet-stream").unwrap());
            multipart = multipart
                .singlepart(Attachment::new(filename.clone()).body(bytes.clone(), content_type));
        }
        builder.multipart(multipart)
    }
    .map_err(|e| SkimError::other("send", format!("cannot build message: {e}")))?;
    Ok(message.formatted())
}

/// Build an iMIP RSVP message: text/plain + text/calendar (method=REPLY)
/// alternative, which organizers (Google, Outlook) auto-process.
pub fn build_calendar_reply(
    account: &Account,
    to: &str,
    subject: &str,
    text_body: &str,
    ics: &str,
) -> Result<Vec<u8>> {
    let from: Mailbox = match &account.display_name {
        Some(name) if !name.is_empty() => format!("{} <{}>", name, account.email),
        _ => account.email.clone(),
    }
    .parse()
    .map_err(|e| SkimError::other("send", format!("invalid sender: {e}")))?;

    let calendar_type = ContentType::parse("text/calendar; charset=utf-8; method=REPLY")
        .map_err(|e| SkimError::other("send", format!("cannot build message: {e}")))?;
    let message = Message::builder()
        .from(from)
        .to(to
            .parse()
            .map_err(|e| SkimError::other("send", format!("invalid recipient {to}: {e}")))?)
        .subject(subject)
        .multipart(
            MultiPart::alternative()
                .singlepart(SinglePart::plain(text_body.to_string()))
                .singlepart(
                    SinglePart::builder()
                        .header(calendar_type)
                        .body(ics.to_string()),
                ),
        )
        .map_err(|e| SkimError::other("send", format!("cannot build message: {e}")))?;
    Ok(message.formatted())
}

/// Build a minimal unsubscribe email (RFC 2369 mailto: path): an empty-bodied
/// message to the list's unsubscribe address. Subject/recipient come from the
/// `List-Unsubscribe` header, so they are protocol values, not UI copy.
pub fn build_unsubscribe_mail(account: &Account, to: &str, subject: &str) -> Result<Vec<u8>> {
    let from: Mailbox = match &account.display_name {
        Some(name) if !name.is_empty() => format!("{} <{}>", name, account.email),
        _ => account.email.clone(),
    }
    .parse()
    .map_err(|e| SkimError::other("send", format!("invalid sender: {e}")))?;

    let message = Message::builder()
        .from(from)
        .to(to
            .parse()
            .map_err(|e| SkimError::other("send", format!("invalid recipient {to}: {e}")))?)
        .subject(subject)
        .body(String::new())
        .map_err(|e| SkimError::other("send", format!("cannot build message: {e}")))?;
    Ok(message.formatted())
}

/// Submit raw MIME over SMTP.
pub async fn send(account: &Account, credentials: &Credentials, raw: &[u8]) -> Result<()> {
    let mut builder = match account.smtp_security.as_str() {
        "tls" => AsyncSmtpTransport::<Tokio1Executor>::relay(&account.smtp_host),
        _ => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&account.smtp_host),
    }
    .map_err(|e| SkimError::other("send", format!("SMTP setup failed: {e}")))?
    .port(account.smtp_port);

    builder = match credentials {
        Credentials::Password(password) => builder.credentials(SmtpCredentials::new(
            account.email.clone(),
            password.clone(),
        )),
        Credentials::OauthToken(token) => builder
            .credentials(SmtpCredentials::new(account.email.clone(), token.clone()))
            .authentication(vec![Mechanism::Xoauth2]),
    };

    let transport = builder.build();

    let envelope = envelope_from_raw(account, raw)?;
    tokio::time::timeout(
        std::time::Duration::from_secs(60),
        transport.send_raw(&envelope, raw),
    )
    .await
    .map_err(|_| SkimError::other("network", "SMTP send timed out"))?
    .map_err(map_smtp_error)?;
    Ok(())
}

fn envelope_from_raw(account: &Account, raw: &[u8]) -> Result<lettre::address::Envelope> {
    // Re-parse recipients out of the built message headers.
    let parsed = mail_parser::MessageParser::default()
        .parse_headers(raw)
        .ok_or_else(|| SkimError::other("send", "cannot parse outgoing message"))?;
    let mut rcpt = Vec::new();
    for header in [parsed.to(), parsed.cc(), parsed.bcc()]
        .into_iter()
        .flatten()
    {
        match header {
            mail_parser::Address::List(list) => {
                for a in list {
                    if let Some(addr) = &a.address {
                        rcpt.push(
                            addr.parse()
                                .map_err(|e| SkimError::other("send", format!("{e}")))?,
                        );
                    }
                }
            }
            mail_parser::Address::Group(groups) => {
                for g in groups {
                    for a in &g.addresses {
                        if let Some(addr) = &a.address {
                            rcpt.push(
                                addr.parse()
                                    .map_err(|e| SkimError::other("send", format!("{e}")))?,
                            );
                        }
                    }
                }
            }
        }
    }
    let from = account
        .email
        .parse()
        .map_err(|e| SkimError::other("send", format!("{e}")))?;
    lettre::address::Envelope::new(Some(from), rcpt)
        .map_err(|e| SkimError::other("send", format!("{e}")))
}

fn map_smtp_error(e: lettre::transport::smtp::Error) -> SkimError {
    let msg = e.to_string();
    if e.is_permanent() {
        SkimError::other("send", format!("server rejected the message: {msg}"))
    } else if e.is_client() || e.is_response() {
        SkimError::other("send", msg)
    } else {
        SkimError::other("network", msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> Account {
        Account {
            id: "acct".into(),
            email: "me@example.com".into(),
            display_name: Some("Me".into()),
            provider: "generic".into(),
            imap_host: "imap.example.com".into(),
            imap_port: 993,
            smtp_host: "smtp.example.com".into(),
            smtp_port: 587,
            smtp_security: "tls".into(),
            auth_kind: "password".into(),
        }
    }

    fn draft() -> Draft {
        Draft {
            id: 1,
            account_id: "acct".into(),
            reply_to_message_id: None,
            mode: "new".into(),
            to: "you@example.com".into(),
            cc: String::new(),
            bcc: String::new(),
            subject: "Hi".into(),
            body: "Body text".into(),
            origin_message_id: None,
        }
    }

    fn no_refs() -> OutgoingRefs {
        OutgoingRefs {
            in_reply_to: None,
            references: Vec::new(),
        }
    }

    #[test]
    fn plain_message_has_no_multipart() {
        let raw = build_message(&account(), &draft(), &no_refs(), &[], None).unwrap();
        let text = String::from_utf8_lossy(&raw);
        assert!(!text.to_ascii_lowercase().contains("multipart/mixed"));
        assert!(text.contains("Body text"));
    }

    #[test]
    fn attachment_produces_multipart_with_file() {
        let attachments = vec![(
            "report.txt".to_string(),
            "text/plain".to_string(),
            b"hello attachment".to_vec(),
        )];
        let raw = build_message(&account(), &draft(), &no_refs(), &attachments, None).unwrap();
        let text = String::from_utf8_lossy(&raw);
        assert!(text.to_ascii_lowercase().contains("multipart/mixed"));
        // The filename appears in the Content-Disposition of the attachment part.
        assert!(text.contains("report.txt"));
        assert!(text.contains("Body text"));
    }

    #[test]
    fn bad_mime_falls_back_to_octet_stream() {
        let attachments = vec![(
            "blob.bin".to_string(),
            "not a mime type".to_string(),
            vec![0u8, 1, 2, 3],
        )];
        // Must not panic on an unparseable MIME type.
        let raw = build_message(&account(), &draft(), &no_refs(), &attachments, None).unwrap();
        let text = String::from_utf8_lossy(&raw);
        assert!(text.contains("blob.bin"));
    }

    #[test]
    fn message_id_override_is_used() {
        let raw = build_message(
            &account(),
            &draft(),
            &no_refs(),
            &[],
            Some("skim-abc@example.com"),
        )
        .unwrap();
        let text = String::from_utf8_lossy(&raw);
        // lettre wraps the id in angle brackets on the Message-ID header line.
        assert!(text.contains("<skim-abc@example.com>"));
    }
}
