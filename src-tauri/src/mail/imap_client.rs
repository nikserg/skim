//! IMAP connection setup: implicit TLS (:993) via rustls with the Windows
//! certificate store, LOGIN or XOAUTH2 authentication.

use crate::error::{Result, SkimError};
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

pub type ImapStream = tokio_rustls::client::TlsStream<TcpStream>;
pub type Session = async_imap::Session<ImapStream>;

#[derive(Debug, Clone)]
pub enum Credentials {
    Password(String),
    /// XOAUTH2 access token (already refreshed).
    OauthToken(String),
}

struct Xoauth2<'a> {
    user: &'a str,
    token: &'a str,
    sent: bool,
}

impl async_imap::Authenticator for Xoauth2<'_> {
    type Response = String;
    fn process(&mut self, _challenge: &[u8]) -> Self::Response {
        if self.sent {
            // On failure the server sends a JSON error as a second challenge
            // and waits for an empty response before returning the tagged NO;
            // answering with the auth string again would deadlock the session.
            String::new()
        } else {
            self.sent = true;
            format!("user={}\x01auth=Bearer {}\x01\x01", self.user, self.token)
        }
    }
}

fn tls_connector() -> Result<TlsConnector> {
    use rustls_platform_verifier::BuilderVerifierExt;
    let config = rustls::ClientConfig::builder()
        .with_platform_verifier()
        .with_no_client_auth();
    Ok(TlsConnector::from(Arc::new(config)))
}

const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
const LOGIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

pub async fn connect(host: &str, port: u16) -> Result<async_imap::Client<ImapStream>> {
    let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect((host, port)))
        .await
        .map_err(|_| SkimError::other("network", format!("connection to {host}:{port} timed out")))?
        .map_err(|e| SkimError::other("network", format!("cannot reach {host}:{port}: {e}")))?;
    tcp.set_nodelay(true).ok();

    let server_name = rustls::pki_types::ServerName::try_from(host.to_string())
        .map_err(|e| SkimError::other("tls", e.to_string()))?;
    let tls = tokio::time::timeout(CONNECT_TIMEOUT, tls_connector()?.connect(server_name, tcp))
        .await
        .map_err(|_| SkimError::other("tls", format!("TLS handshake with {host} timed out")))?
        .map_err(|e| SkimError::other("tls", format!("TLS handshake with {host} failed: {e}")))?;

    let mut client = async_imap::Client::new(tls);
    // Consume the server greeting ("* OK ... ready"). Leaving it unread
    // desyncs AUTHENTICATE's state machine: the '+' continuation gets
    // treated as unsolicited and both sides wait on each other forever.
    tokio::time::timeout(CONNECT_TIMEOUT, client.read_response())
        .await
        .map_err(|_| SkimError::other("network", format!("{host} sent no greeting")))?
        .transpose()
        .map_err(|e| SkimError::other("imap", format!("bad greeting from {host}: {e}")))?
        .ok_or_else(|| SkimError::other("network", format!("{host} closed the connection")))?;

    Ok(client)
}

pub async fn login(
    host: &str,
    port: u16,
    user: &str,
    credentials: &Credentials,
) -> Result<Session> {
    let client = connect(host, port).await?;
    let attempt = async {
        match credentials {
            Credentials::Password(password) => client
                .login(user, password)
                .await
                .map_err(|(e, _)| SkimError::other("auth", format!("sign-in failed: {e}"))),
            Credentials::OauthToken(token) => {
                let auth = Xoauth2 {
                    user,
                    token,
                    sent: false,
                };
                client
                    .authenticate("XOAUTH2", auth)
                    .await
                    .map_err(|(e, _)| oauth_login_error(e))
            }
        }
    };
    tokio::time::timeout(LOGIN_TIMEOUT, attempt)
        .await
        .map_err(|_| SkimError::other("auth", format!("{host} did not answer the sign-in")))?
}

/// Turn an XOAUTH2 failure into a user-facing error. Exchange Online answers a
/// perfectly valid token with "User is authenticated but not connected." when
/// the mailbox has IMAP switched off — Outlook.com ships it off by default and
/// the user must enable it (Settings → Mail → Forwarding and IMAP). The token,
/// scopes, and username are all correct, so flag this with a dedicated
/// `imap_disabled` code the UI turns into a fix-it prompt with a retry.
fn oauth_login_error(e: impl std::fmt::Display) -> SkimError {
    let msg = e.to_string();
    if msg.contains("authenticated but not connected") {
        SkimError::other(
            "imap_disabled",
            "IMAP is turned off for this mailbox. Enable it in your Outlook \
             account settings, then try again.",
        )
    } else {
        SkimError::other("auth", format!("sign-in failed: {msg}"))
    }
}

#[cfg(test)]
mod tests {
    use super::oauth_login_error;

    #[test]
    fn mailbox_refusal_becomes_a_hint() {
        // The exact shape async-imap surfaces for the Exchange NO response.
        let e = oauth_login_error(
            "no response: code: None, info: Some(\"User is authenticated but not connected.\")",
        );
        assert_eq!(e.code(), "imap_disabled");
        assert!(e.to_string().contains("IMAP"), "{e}");
    }

    #[test]
    fn other_auth_errors_pass_through() {
        let e = oauth_login_error("LOGIN failed.");
        assert!(e.to_string().contains("sign-in failed"));
    }
}
