use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use super::dns;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerPreset {
    pub provider: &'static str,
    pub imap_host: &'static str,
    pub imap_port: u16,
    pub smtp_host: &'static str,
    pub smtp_port: u16,
    pub smtp_security: &'static str, // 'starttls' | 'tls'
    /// Whether the provider requires an app password (2FA) for IMAP.
    pub needs_app_password: bool,
    pub supports_oauth: bool,
}

/// Outlook.com, Hotmail, Live — a personal mailbox on Microsoft's consumer edge.
const OUTLOOK_CONSUMER: ServerPreset = ServerPreset {
    provider: "outlook",
    imap_host: "outlook.office365.com",
    imap_port: 993,
    smtp_host: "smtp-mail.outlook.com",
    smtp_port: 587,
    smtp_security: "starttls",
    needs_app_password: true,
    supports_oauth: true,
};

/// A work or school mailbox in an Exchange Online tenant. Office 365's own SMTP
/// host, and no app-password hint: Microsoft is retiring Basic Auth there, so
/// pointing at app passwords would be pointing at a dead end.
const EXCHANGE_ONLINE: ServerPreset = ServerPreset {
    provider: "outlook",
    imap_host: "outlook.office365.com",
    imap_port: 993,
    smtp_host: "smtp.office365.com",
    smtp_port: 587,
    smtp_security: "starttls",
    needs_app_password: false,
    supports_oauth: true,
};

/// Well-known server settings by mail domain.
pub fn lookup(email: &str) -> Option<ServerPreset> {
    let domain = email.rsplit('@').next()?.to_lowercase();
    let preset = match domain.as_str() {
        "gmail.com" | "googlemail.com" => ServerPreset {
            provider: "gmail",
            imap_host: "imap.gmail.com",
            imap_port: 993,
            smtp_host: "smtp.gmail.com",
            smtp_port: 587,
            smtp_security: "starttls",
            needs_app_password: true,
            supports_oauth: true,
        },
        "outlook.com" | "hotmail.com" | "live.com" | "msn.com" => OUTLOOK_CONSUMER,
        "yahoo.com" => ServerPreset {
            provider: "yahoo",
            imap_host: "imap.mail.yahoo.com",
            imap_port: 993,
            smtp_host: "smtp.mail.yahoo.com",
            smtp_port: 465,
            smtp_security: "tls",
            needs_app_password: true,
            supports_oauth: false,
        },
        "icloud.com" | "me.com" | "mac.com" => ServerPreset {
            provider: "icloud",
            imap_host: "imap.mail.me.com",
            imap_port: 993,
            smtp_host: "smtp.mail.me.com",
            smtp_port: 587,
            smtp_security: "starttls",
            needs_app_password: true,
            supports_oauth: false,
        },
        _ => return None,
    };
    Some(preset)
}

/// Which kind of Microsoft mailbox a domain's MX records point at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MicrosoftKind {
    Consumer,
    ExchangeOnline,
}

/// Microsoft-hosted mail always lands on `*.protection.outlook.com`. Consumer
/// mailboxes on country domains (hotmail.de, outlook.fr …) come in through the
/// Outlook.com edge, `*.olc.protection.outlook.com`; everything else on that
/// suffix is an Exchange Online tenant (`contoso-com.mail.protection.outlook.com`).
fn microsoft_mx_kind<S: AsRef<str>>(hosts: &[S]) -> Option<MicrosoftKind> {
    let mut kind = None;
    for host in hosts {
        let host = host.as_ref().trim_end_matches('.').to_ascii_lowercase();
        // The leading dot matters: `notprotection.outlook.com` isn't Microsoft's.
        if !host.ends_with(".protection.outlook.com") {
            continue;
        }
        if host.ends_with(".olc.protection.outlook.com") {
            kind = Some(MicrosoftKind::Consumer);
        } else {
            // A tenant outranks the consumer edge if a domain lists both.
            return Some(MicrosoftKind::ExchangeOnline);
        }
    }
    kind
}

fn preset_for(kind: MicrosoftKind) -> ServerPreset {
    match kind {
        MicrosoftKind::Consumer => OUTLOOK_CONSUMER,
        MicrosoftKind::ExchangeOnline => EXCHANGE_ONLINE,
    }
}

/// Cheap sanity gate so a half-typed address never reaches the resolver.
fn looks_like_domain(domain: &str) -> bool {
    !domain.is_empty()
        && domain.len() < 254
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains("..")
        && domain
            .chars()
            .all(|c| !c.is_whitespace() && c != '@' && c != ',')
}

/// How long the connect screen waits on DNS before falling back to the manual
/// form. `DnsQuery_W` has no timeout of its own and the user is watching.
const MX_TIMEOUT: Duration = Duration::from_secs(3);

/// Per-domain memo for the life of the process — the connect screen asks again
/// on every blur of the email field, and the system DNS cache does the rest.
fn mx_cache() -> &'static Mutex<HashMap<String, Option<MicrosoftKind>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<MicrosoftKind>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Well-known settings first (instant, offline); on a miss, ask DNS who runs the
/// mail for this domain. Microsoft answers for far more domains than any table
/// can list: every Exchange Online tenant, plus the country-specific consumer
/// domains (hotmail.de, outlook.fr …).
pub async fn lookup_async(email: &str) -> Option<ServerPreset> {
    if let Some(preset) = lookup(email) {
        return Some(preset);
    }
    let domain = email.rsplit('@').next()?.to_ascii_lowercase();
    if !looks_like_domain(&domain) {
        return None;
    }
    if let Some(cached) = mx_cache()
        .lock()
        .ok()
        .and_then(|cache| cache.get(&domain).copied())
    {
        return cached.map(preset_for);
    }

    let probe = domain.clone();
    let kind = match tokio::time::timeout(
        MX_TIMEOUT,
        tokio::task::spawn_blocking(move || microsoft_mx_kind(&dns::mx_hosts(&probe))),
    )
    .await
    {
        Ok(Ok(kind)) => kind,
        // Timed out, or the blocking task died: behave exactly as this screen did
        // before the probe existed, and don't cache a non-answer.
        _ => return None,
    };
    if let Ok(mut cache) = mx_cache().lock() {
        cache.insert(domain, kind);
    }
    kind.map(preset_for)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_offline_table_still_answers_the_domains_it_always_did() {
        let gmail = lookup("a@gmail.com").expect("gmail preset");
        assert_eq!(gmail.provider, "gmail");
        assert_eq!(gmail.imap_host, "imap.gmail.com");

        let outlook = lookup("a@Outlook.com").expect("outlook preset");
        assert_eq!(outlook.provider, "outlook");
        assert_eq!(outlook.smtp_host, "smtp-mail.outlook.com");

        assert!(lookup("a@contoso.com").is_none());
    }

    #[test]
    fn a_tenant_mx_is_exchange_online() {
        assert_eq!(
            microsoft_mx_kind(&["contoso-com.mail.protection.outlook.com."]),
            Some(MicrosoftKind::ExchangeOnline)
        );
    }

    #[test]
    fn a_country_hotmail_mx_is_a_consumer_mailbox() {
        assert_eq!(
            microsoft_mx_kind(&["eur.olc.protection.outlook.com"]),
            Some(MicrosoftKind::Consumer)
        );
    }

    #[test]
    fn mx_hosts_match_whatever_their_case_and_trailing_dot() {
        assert_eq!(
            microsoft_mx_kind(&["EUR.OLC.PROTECTION.OUTLOOK.COM."]),
            Some(MicrosoftKind::Consumer)
        );
    }

    #[test]
    fn a_tenant_outranks_a_consumer_edge_when_both_are_listed() {
        assert_eq!(
            microsoft_mx_kind(&[
                "eur.olc.protection.outlook.com",
                "contoso-com.mail.protection.outlook.com",
            ]),
            Some(MicrosoftKind::ExchangeOnline)
        );
    }

    #[test]
    fn lookalike_and_foreign_mx_hosts_are_not_microsoft() {
        assert_eq!(microsoft_mx_kind(&["notprotection.outlook.com"]), None);
        assert_eq!(
            microsoft_mx_kind(&["mail.protection.outlook.com.evil.example"]),
            None
        );
        assert_eq!(
            microsoft_mx_kind(&["aspmx.l.google.com", "mx1.example.pphosted.com"]),
            None
        );
        assert_eq!(microsoft_mx_kind::<&str>(&[]), None);
    }

    #[test]
    fn the_exchange_online_preset_leads_with_oauth() {
        let preset = preset_for(MicrosoftKind::ExchangeOnline);
        // `outlook` is what makes the connect screen lead with the Microsoft button.
        assert_eq!(preset.provider, "outlook");
        assert_eq!(preset.imap_host, "outlook.office365.com");
        assert_eq!(preset.smtp_host, "smtp.office365.com");
        assert!(preset.supports_oauth);
        assert!(!preset.needs_app_password);
    }

    #[test]
    fn half_typed_addresses_never_reach_the_resolver() {
        assert!(looks_like_domain("contoso.com"));
        assert!(!looks_like_domain(""));
        assert!(!looks_like_domain("contoso"));
        assert!(!looks_like_domain("contoso."));
        assert!(!looks_like_domain(".contoso.com"));
        assert!(!looks_like_domain("con toso.com"));
        assert!(!looks_like_domain("contoso..com"));
    }
}
