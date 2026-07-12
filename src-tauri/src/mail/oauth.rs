//! Google OAuth for Gmail IMAP/SMTP (XOAUTH2).
//!
//! Installed-app flow: loopback redirect on 127.0.0.1 with PKCE (S256).
//! The client id/secret are either baked in at build time via the
//! `SKIM_GOOGLE_CLIENT_ID` / `SKIM_GOOGLE_CLIENT_SECRET` env vars or supplied
//! by the user in settings (stored in the `settings` table — they are not
//! secret for installed apps, per Google's own documentation).

use crate::error::{Result, SkimError};
use base64::Engine;
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const USERINFO_ENDPOINT: &str = "https://openidconnect.googleapis.com/v1/userinfo";
const SCOPES: &str = "https://mail.google.com/ openid email";

#[derive(Debug, Clone)]
pub struct OauthConfig {
    pub client_id: String,
    pub client_secret: String,
}

pub fn baked_in_config() -> Option<OauthConfig> {
    let id = option_env!("SKIM_GOOGLE_CLIENT_ID")?;
    if id.is_empty() {
        return None;
    }
    Some(OauthConfig {
        client_id: id.to_string(),
        client_secret: option_env!("SKIM_GOOGLE_CLIENT_SECRET")
            .unwrap_or_default()
            .to_string(),
    })
}

#[derive(Debug, Clone)]
pub struct OauthOutcome {
    pub email: String,
    pub refresh_token: String,
    pub access_token: String,
    /// Unix seconds when the access token expires.
    pub expires_at: i64,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

#[derive(Deserialize)]
struct UserInfo {
    email: String,
}

fn random_token() -> String {
    // 32 bytes of OS randomness, URL-safe base64.
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).expect("OS randomness unavailable");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

fn sha256_base64url(input: &str) -> String {
    // PKCE S256 challenge.
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(input.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash)
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Run the full authorization flow: open the system browser, wait for the
/// loopback redirect, exchange the code, and resolve the account email.
pub async fn authorize(
    config: &OauthConfig,
    open_url: impl FnOnce(&str) -> Result<()>,
) -> Result<OauthOutcome> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|e| SkimError::other("oauth", format!("cannot bind loopback port: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| SkimError::other("oauth", e.to_string()))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}");

    let verifier = format!("{}{}", random_token(), random_token());
    let challenge = sha256_base64url(&verifier);
    let state = random_token();

    let mut auth_url = url::Url::parse(AUTH_ENDPOINT).expect("static url");
    auth_url
        .query_pairs_mut()
        .append_pair("client_id", &config.client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", SCOPES)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state)
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent");

    open_url(auth_url.as_str())?;

    // Wait for the browser redirect (10 minutes budget).
    let code = tokio::time::timeout(
        std::time::Duration::from_secs(600),
        wait_for_code(listener, &state),
    )
    .await
    .map_err(|_| SkimError::other("oauth", "authorization timed out"))??;

    // Exchange the code.
    let client = reqwest::Client::new();
    let mut form = vec![
        ("client_id", config.client_id.clone()),
        ("code", code),
        ("code_verifier", verifier),
        ("grant_type", "authorization_code".into()),
        ("redirect_uri", redirect_uri),
    ];
    if !config.client_secret.is_empty() {
        form.push(("client_secret", config.client_secret.clone()));
    }
    let resp = client
        .post(TOKEN_ENDPOINT)
        .form(&form)
        .send()
        .await
        .map_err(|e| SkimError::other("network", e.to_string()))?;
    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(SkimError::other(
            "oauth",
            format!("token exchange failed: {body}"),
        ));
    }
    let tokens: TokenResponse = resp
        .json()
        .await
        .map_err(|e| SkimError::other("oauth", e.to_string()))?;
    let refresh_token = tokens.refresh_token.ok_or_else(|| {
        SkimError::other(
            "oauth",
            "Google did not return a refresh token; remove Skim from your Google account's \
             third-party access list and try again",
        )
    })?;

    // Resolve the email address.
    let userinfo: UserInfo = client
        .get(USERINFO_ENDPOINT)
        .bearer_auth(&tokens.access_token)
        .send()
        .await
        .map_err(|e| SkimError::other("network", e.to_string()))?
        .json()
        .await
        .map_err(|e| SkimError::other("oauth", e.to_string()))?;

    Ok(OauthOutcome {
        email: userinfo.email,
        refresh_token,
        access_token: tokens.access_token,
        expires_at: now_unix() + tokens.expires_in.unwrap_or(3600) - 60,
    })
}

/// Exchange a refresh token for a fresh access token.
pub async fn refresh_access_token(
    config: &OauthConfig,
    refresh_token: &str,
) -> Result<(String, i64)> {
    let client = reqwest::Client::new();
    let mut form = vec![
        ("client_id", config.client_id.clone()),
        ("refresh_token", refresh_token.to_string()),
        ("grant_type", "refresh_token".into()),
    ];
    if !config.client_secret.is_empty() {
        form.push(("client_secret", config.client_secret.clone()));
    }
    let resp = client
        .post(TOKEN_ENDPOINT)
        .form(&form)
        .send()
        .await
        .map_err(|e| SkimError::other("network", e.to_string()))?;
    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        let code = if body.contains("invalid_grant") {
            "oauth_expired"
        } else {
            "oauth"
        };
        return Err(SkimError::other(
            code,
            format!("token refresh failed: {body}"),
        ));
    }
    let tokens: TokenResponse = resp
        .json()
        .await
        .map_err(|e| SkimError::other("oauth", e.to_string()))?;
    Ok((
        tokens.access_token,
        now_unix() + tokens.expires_in.unwrap_or(3600) - 60,
    ))
}

async fn wait_for_code(listener: TcpListener, expected_state: &str) -> Result<String> {
    loop {
        let (mut stream, _) = listener
            .accept()
            .await
            .map_err(|e| SkimError::other("oauth", e.to_string()))?;
        let mut buf = vec![0u8; 8192];
        let n = stream.read(&mut buf).await.unwrap_or(0);
        let request = String::from_utf8_lossy(&buf[..n]);

        // First line: GET /?code=...&state=... HTTP/1.1
        let path = request
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or("/");
        let url = url::Url::parse(&format!("http://localhost{path}"))
            .map_err(|e| SkimError::other("oauth", e.to_string()))?;
        let get = |k: &str| {
            url.query_pairs()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.to_string())
        };

        let ok = get("state").as_deref() == Some(expected_state) && get("code").is_some();
        let (status, body) = if ok {
            (
                "200 OK",
                "<html><body style=\"font-family:sans-serif;text-align:center;padding-top:20vh\">\
                 <h2>Skim is connected</h2><p>You can close this tab and return to the app.</p>\
                 </body></html>",
            )
        } else if get("error").is_some() {
            ("200 OK", "<html><body style=\"font-family:sans-serif;text-align:center;padding-top:20vh\"><h2>Authorization was cancelled</h2><p>You can close this tab.</p></body></html>")
        } else {
            // Favicon or stray request — keep listening.
            let _ = stream
                .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                .await;
            continue;
        };
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.shutdown().await;

        return match (get("code"), get("error")) {
            (Some(code), _) if get("state").as_deref() == Some(expected_state) => Ok(code),
            (_, Some(err)) => Err(SkimError::other("oauth_cancelled", err)),
            _ => Err(SkimError::other("oauth", "state mismatch in redirect")),
        };
    }
}
