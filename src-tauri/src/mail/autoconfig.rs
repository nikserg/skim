use serde::Serialize;

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
        "outlook.com" | "hotmail.com" | "live.com" | "msn.com" => ServerPreset {
            provider: "outlook",
            imap_host: "outlook.office365.com",
            imap_port: 993,
            smtp_host: "smtp-mail.outlook.com",
            smtp_port: 587,
            smtp_security: "starttls",
            needs_app_password: true,
            supports_oauth: true,
        },
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
