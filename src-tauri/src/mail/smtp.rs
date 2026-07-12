//! Outgoing mail: MIME construction and SMTP submission via lettre.

use crate::db::drafts::Draft;
use crate::db::models::Account;
use crate::error::{Result, SkimError};
use crate::mail::imap_client::Credentials;
use lettre::message::{Mailbox, Message};
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
pub fn build_message(account: &Account, draft: &Draft, refs: &OutgoingRefs) -> Result<Vec<u8>> {
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

    if let Some(irt) = &refs.in_reply_to {
        builder = builder.in_reply_to(irt.clone());
    }
    if !refs.references.is_empty() {
        builder = builder.references(refs.references.join(" "));
    }

    let message = builder
        .body(draft.body.clone())
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
