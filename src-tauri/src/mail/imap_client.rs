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
}

impl async_imap::Authenticator for Xoauth2<'_> {
    type Response = String;
    fn process(&mut self, _challenge: &[u8]) -> Self::Response {
        format!("user={}\x01auth=Bearer {}\x01\x01", self.user, self.token)
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
        .map_err(|_| {
            SkimError::other("network", format!("connection to {host}:{port} timed out"))
        })?
        .map_err(|e| SkimError::other("network", format!("cannot reach {host}:{port}: {e}")))?;
    tcp.set_nodelay(true).ok();

    let server_name = rustls::pki_types::ServerName::try_from(host.to_string())
        .map_err(|e| SkimError::other("tls", e.to_string()))?;
    let tls = tokio::time::timeout(CONNECT_TIMEOUT, tls_connector()?.connect(server_name, tcp))
        .await
        .map_err(|_| SkimError::other("tls", format!("TLS handshake with {host} timed out")))?
        .map_err(|e| SkimError::other("tls", format!("TLS handshake with {host} failed: {e}")))?;

    Ok(async_imap::Client::new(tls))
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
                let auth = Xoauth2 { user, token };
                client
                    .authenticate("XOAUTH2", auth)
                    .await
                    .map_err(|(e, _)| SkimError::other("auth", format!("sign-in failed: {e}")))
            }
        }
    };
    tokio::time::timeout(LOGIN_TIMEOUT, attempt)
        .await
        .map_err(|_| SkimError::other("auth", format!("{host} did not answer the sign-in")))?
}
