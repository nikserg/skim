//! IMAP connection setup: implicit TLS (:993) or STARTTLS (:143), picked
//! automatically from the port — rustls with the Windows certificate store,
//! LOGIN or XOAUTH2 authentication. Credentials never travel unencrypted.

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

/// Port 143 is the plaintext IMAP port that upgrades via STARTTLS; every
/// other port gets implicit TLS (993-style) from the first byte.
fn uses_starttls(port: u16) -> bool {
    port == 143
}

async fn tcp_connect(host: &str, port: u16) -> Result<TcpStream> {
    let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect((host, port)))
        .await
        .map_err(|_| SkimError::other("network", format!("connection to {host}:{port} timed out")))?
        .map_err(|e| SkimError::other("network", format!("cannot reach {host}:{port}: {e}")))?;
    tcp.set_nodelay(true).ok();
    Ok(tcp)
}

async fn tls_handshake(host: &str, port: u16, tcp: TcpStream) -> Result<ImapStream> {
    let server_name = rustls::pki_types::ServerName::try_from(host.to_string())
        .map_err(|e| SkimError::other("tls", e.to_string()))?;
    tokio::time::timeout(CONNECT_TIMEOUT, tls_connector()?.connect(server_name, tcp))
        .await
        .map_err(|_| SkimError::other("tls", format!("TLS handshake with {host} timed out")))?
        .map_err(|e| tls_handshake_error(host, port, e))
}

/// Consume the server greeting ("* OK ... ready"). Leaving it unread
/// desyncs AUTHENTICATE's state machine: the '+' continuation gets
/// treated as unsolicited and both sides wait on each other forever.
async fn read_greeting<T>(client: &mut async_imap::Client<T>, host: &str) -> Result<()>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    tokio::time::timeout(CONNECT_TIMEOUT, client.read_response())
        .await
        .map_err(|_| SkimError::other("network", format!("{host} sent no greeting")))?
        .transpose()
        .map_err(|e| SkimError::other("imap", format!("bad greeting from {host}: {e}")))?
        .ok_or_else(|| SkimError::other("network", format!("{host} closed the connection")))?;
    Ok(())
}

pub async fn connect(host: &str, port: u16) -> Result<async_imap::Client<ImapStream>> {
    if uses_starttls(port) {
        connect_starttls(host, port).await
    } else {
        connect_tls(host, port).await
    }
}

async fn connect_tls(host: &str, port: u16) -> Result<async_imap::Client<ImapStream>> {
    let tcp = tcp_connect(host, port).await?;
    let tls = tls_handshake(host, port, tcp).await?;
    let mut client = async_imap::Client::new(tls);
    read_greeting(&mut client, host).await?;
    Ok(client)
}

async fn connect_starttls(host: &str, port: u16) -> Result<async_imap::Client<ImapStream>> {
    let tcp = tcp_connect(host, port).await?;
    // Plaintext phase: greeting + STARTTLS only. Nothing sensitive is sent
    // before the TLS handshake completes; any failure aborts the connection.
    let mut plain = async_imap::Client::new(tcp);
    read_greeting(&mut plain, host).await?;
    tokio::time::timeout(
        CONNECT_TIMEOUT,
        plain.run_command_and_check_ok("STARTTLS", None),
    )
    .await
    .map_err(|_| SkimError::other("network", format!("{host} did not answer STARTTLS")))?
    .map_err(|e| starttls_error(host, e))?;
    // into_inner drops the read buffer — safe: after the tagged OK the server
    // stays silent until the client opens the TLS handshake.
    let tls = tls_handshake(host, port, plain.into_inner()).await?;
    // No second greeting after STARTTLS.
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

/// The server on port 143 refused the STARTTLS upgrade. Skim never falls
/// back to plaintext — surface a fix-it error pointing at port 993 instead.
fn starttls_error(host: &str, e: async_imap::error::Error) -> SkimError {
    use async_imap::error::Error;
    match e {
        Error::No(_) | Error::Bad(_) => SkimError::other(
            "starttls_unsupported",
            format!(
                "{host} does not offer encryption (STARTTLS) on port 143, and Skim \
                 never signs in over an unencrypted connection. Try IMAP port 993."
            ),
        ),
        other => SkimError::other("imap", format!("STARTTLS with {host} failed: {other}")),
    }
}

/// A TLS handshake dying with rustls's InvalidContentType almost always means
/// the server answered in plaintext — the port is not a TLS port. Anything
/// else (certificate problems, resets) stays a plain "tls" error.
fn tls_handshake_error(host: &str, port: u16, e: std::io::Error) -> SkimError {
    let msg = e.to_string();
    if msg.contains("InvalidContentType") || msg.contains("corrupt message") {
        SkimError::other(
            "plaintext_port",
            format!(
                "{host}:{port} answered without encryption — this doesn't look like \
                 a TLS port. Most providers use IMAP port 993; some use 143."
            ),
        )
    } else {
        SkimError::other("tls", format!("TLS handshake with {host} failed: {msg}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn starttls_only_on_143() {
        assert!(uses_starttls(143));
        assert!(!uses_starttls(993));
        assert!(!uses_starttls(1993));
    }

    #[test]
    fn starttls_refusal_names_the_fix() {
        let e = starttls_error(
            "mail.example.com",
            async_imap::error::Error::Bad("unknown command".into()),
        );
        assert_eq!(e.code(), "starttls_unsupported");
        assert!(e.to_string().contains("993"), "{e}");
    }

    #[test]
    fn starttls_io_errors_stay_generic() {
        let io = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset");
        let e = starttls_error("mail.example.com", async_imap::error::Error::Io(io));
        assert_eq!(e.code(), "imap");
    }

    #[test]
    fn plaintext_server_on_tls_port_gets_a_hint() {
        let io = std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "received corrupt message of type InvalidContentType",
        );
        let e = tls_handshake_error("mail.example.com", 143, io);
        assert_eq!(e.code(), "plaintext_port");
        assert!(e.to_string().contains("993"), "{e}");
    }

    #[test]
    fn other_tls_failures_pass_through() {
        let io = std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid peer certificate: Expired",
        );
        assert_eq!(
            tls_handshake_error("mail.example.com", 993, io).code(),
            "tls"
        );
    }

    /// Scripted server that refuses STARTTLS: the upgrade must fail with the
    /// fix-it code, never continue in plaintext.
    #[tokio::test]
    async fn refused_starttls_maps_to_fix_it_error() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (sock, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(sock);
            reader.get_mut().write_all(b"* OK ready\r\n").await.unwrap();
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            let tag = line.split_whitespace().next().unwrap_or("a1").to_string();
            reader
                .get_mut()
                .write_all(format!("{tag} NO not supported\r\n").as_bytes())
                .await
                .unwrap();
        });
        let err = connect_starttls("127.0.0.1", port).await.unwrap_err();
        assert_eq!(err.code(), "starttls_unsupported", "{err}");
        server.await.unwrap();
    }

    /// Direct repro of issue #19: a plaintext IMAP greeting where a TLS
    /// handshake was expected must produce the friendly hint, not a cryptic
    /// InvalidContentType error.
    #[tokio::test]
    async fn plaintext_greeting_on_tls_path_maps_to_hint() {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            sock.write_all(b"* OK ready\r\n").await.unwrap();
            // Keep the socket open until the client gives up on the handshake.
            let mut buf = [0u8; 512];
            use tokio::io::AsyncReadExt;
            let _ = sock.read(&mut buf).await;
        });
        let err = connect_tls("127.0.0.1", port).await.unwrap_err();
        assert_eq!(err.code(), "plaintext_port", "{err}");
        server.abort();
    }
}
