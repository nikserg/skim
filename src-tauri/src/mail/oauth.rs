//! OAuth (XOAUTH2) for Gmail and Microsoft (Exchange Online / Office 365).
//!
//! Installed-app flow: loopback redirect on 127.0.0.1 with PKCE (S256).
//! The client id/secret are baked in at build time via the
//! `SKIM_GOOGLE_CLIENT_ID` / `SKIM_GOOGLE_CLIENT_SECRET` /
//! `SKIM_MICROSOFT_CLIENT_ID` env vars (they are not secret for installed
//! apps, per Google's and Microsoft's own documentation). Microsoft public
//! clients use PKCE only and carry no secret.

use crate::error::{Result, SkimError};
use base64::Engine;
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const USERINFO_ENDPOINT: &str = "https://openidconnect.googleapis.com/v1/userinfo";

/// The identity provider that issues the OAuth tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OauthProvider {
    Google,
    /// Microsoft `common` authority — both work/school (Office 365) and
    /// personal (outlook.com) accounts.
    Microsoft,
}

impl OauthProvider {
    fn auth_endpoint(self) -> &'static str {
        match self {
            OauthProvider::Google => "https://accounts.google.com/o/oauth2/v2/auth",
            OauthProvider::Microsoft => {
                "https://login.microsoftonline.com/common/oauth2/v2.0/authorize"
            }
        }
    }

    fn token_endpoint(self) -> &'static str {
        match self {
            OauthProvider::Google => "https://oauth2.googleapis.com/token",
            OauthProvider::Microsoft => {
                "https://login.microsoftonline.com/common/oauth2/v2.0/token"
            }
        }
    }

    fn scopes(self) -> &'static str {
        match self {
            OauthProvider::Google => "https://mail.google.com/ openid email",
            OauthProvider::Microsoft => {
                // The OAuth resource for IMAP/SMTP is `outlook.office.com` — the
                // `outlook.office365.com` alias is NOT valid for personal
                // Microsoft accounts and yields `invalid_scope`. (The IMAP/SMTP
                // *server* hosts are still *.office365.com; that's unrelated.)
                "https://outlook.office.com/IMAP.AccessAsUser.All \
                 https://outlook.office.com/SMTP.Send offline_access openid email profile"
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct OauthConfig {
    pub provider: OauthProvider,
    pub client_id: String,
    pub client_secret: String,
}

/// Client credentials baked in at build time, if any, for the given provider.
pub fn baked_in_config(provider: OauthProvider) -> Option<OauthConfig> {
    let (id, secret) = match provider {
        OauthProvider::Google => (
            option_env!("SKIM_GOOGLE_CLIENT_ID"),
            option_env!("SKIM_GOOGLE_CLIENT_SECRET").unwrap_or_default(),
        ),
        // Public client — no secret.
        OauthProvider::Microsoft => (option_env!("SKIM_MICROSOFT_CLIENT_ID"), ""),
    };
    let id = id?;
    if id.is_empty() {
        return None;
    }
    Some(OauthConfig {
        provider,
        client_id: id.to_string(),
        client_secret: secret.to_string(),
    })
}

/// Whether this provider's OAuth app has cleared provider-side verification.
///
/// This does **not** change the flow — it only tells the UI whether to show the
/// "unverified/limited sign-in" caveat. Baked in at build time via an env flag
/// (`"1"`/`"true"`) so official installers can flip it without a code change:
/// - Google stays unverified until a (paid) CASA assessment for the restricted
///   `https://mail.google.com/` scope, so its one-click sign-in is limited
///   (testing mode: weekly re-auth, 100-user cap).
/// - Microsoft flips to verified once the free publisher verification is done.
pub fn oauth_verified(provider: OauthProvider) -> bool {
    let flag = match provider {
        OauthProvider::Google => option_env!("SKIM_GOOGLE_OAUTH_VERIFIED"),
        OauthProvider::Microsoft => option_env!("SKIM_MICROSOFT_OAUTH_VERIFIED"),
    };
    matches!(flag, Some("1") | Some("true"))
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
    /// OpenID Connect id token — Microsoft carries the account address in its
    /// claims, sparing us a separate userinfo/Graph round-trip.
    #[serde(default)]
    id_token: Option<String>,
}

#[derive(Deserialize)]
struct UserInfo {
    email: String,
}

/// Pull the account address out of an OIDC id token's claims (unverified: the
/// token came straight from the provider's TLS token endpoint). Microsoft puts
/// the address in `preferred_username`; fall back to the standard claims.
fn email_from_id_token(id_token: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    ["preferred_username", "email", "upn"]
        .iter()
        .find_map(|k| claims.get(*k).and_then(|v| v.as_str()))
        .map(|s| s.to_string())
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

    let auth_url = build_auth_url(config, &redirect_uri, &challenge, &state);

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
        .post(config.provider.token_endpoint())
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
    let refresh_token = tokens.refresh_token.clone().ok_or_else(|| {
        SkimError::other(
            "oauth",
            match config.provider {
                OauthProvider::Google => {
                    "Google did not return a refresh token; remove Skim from your Google \
                     account's third-party access list and try again"
                }
                OauthProvider::Microsoft => {
                    "Microsoft did not return a refresh token; make sure the offline_access \
                     scope is granted and try again"
                }
            },
        )
    })?;

    let email = resolve_email(&client, config.provider, &tokens).await?;

    Ok(OauthOutcome {
        email,
        refresh_token,
        access_token: tokens.access_token,
        expires_at: now_unix() + tokens.expires_in.unwrap_or(3600) - 60,
    })
}

/// Build the provider-specific authorization URL.
fn build_auth_url(
    config: &OauthConfig,
    redirect_uri: &str,
    challenge: &str,
    state: &str,
) -> url::Url {
    let mut auth_url = url::Url::parse(config.provider.auth_endpoint()).expect("static url");
    {
        let mut q = auth_url.query_pairs_mut();
        q.append_pair("client_id", &config.client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("scope", config.provider.scopes())
            .append_pair("code_challenge", challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", state);
        match config.provider {
            // Force a refresh token and a fresh consent on every run.
            OauthProvider::Google => {
                q.append_pair("access_type", "offline")
                    .append_pair("prompt", "consent");
            }
            // `offline_access` (in the scope) already yields a refresh token;
            // let the user pick which account to sign in with.
            OauthProvider::Microsoft => {
                q.append_pair("prompt", "select_account");
            }
        }
    }
    auth_url
}

/// Resolve the account's email address after a successful token exchange.
async fn resolve_email(
    client: &reqwest::Client,
    provider: OauthProvider,
    tokens: &TokenResponse,
) -> Result<String> {
    match provider {
        OauthProvider::Google => {
            // The consent screen lets users untick individual permissions —
            // verify the mail scope actually made it into the token first.
            let tokeninfo: serde_json::Value = client
                .get(format!(
                    "https://www.googleapis.com/oauth2/v3/tokeninfo?access_token={}",
                    tokens.access_token
                ))
                .send()
                .await
                .map_err(|e| SkimError::other("network", e.to_string()))?
                .json()
                .await
                .unwrap_or_default();
            let scope = tokeninfo["scope"].as_str().unwrap_or_default();
            if !scope.contains("https://mail.google.com/") {
                return Err(SkimError::other(
                    "oauth",
                    "Google did not grant mail access. Make sure the scope \
                     https://mail.google.com/ is added under Data access in your \
                     Google Cloud project, and that you approve it on the consent \
                     screen (it may be an unticked checkbox).",
                ));
            }
            let userinfo: UserInfo = client
                .get(USERINFO_ENDPOINT)
                .bearer_auth(&tokens.access_token)
                .send()
                .await
                .map_err(|e| SkimError::other("network", e.to_string()))?
                .json()
                .await
                .map_err(|e| SkimError::other("oauth", e.to_string()))?;
            Ok(userinfo.email)
        }
        // The address rides along in the id token's claims — no extra call.
        OauthProvider::Microsoft => tokens
            .id_token
            .as_deref()
            .and_then(email_from_id_token)
            .ok_or_else(|| {
                SkimError::other("oauth", "Microsoft did not return an account address")
            }),
    }
}

/// A refreshed access token, plus a rotated refresh token when the provider
/// issues one (Microsoft rotates on every refresh; Google typically doesn't).
#[derive(Debug, Clone)]
pub struct RefreshedToken {
    pub access_token: String,
    /// Unix seconds when the access token expires.
    pub expires_at: i64,
    /// The new refresh token to persist, if the provider returned one.
    pub new_refresh_token: Option<String>,
}

/// Exchange a refresh token for a fresh access token.
pub async fn refresh_access_token(
    config: &OauthConfig,
    refresh_token: &str,
) -> Result<RefreshedToken> {
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
        .post(config.provider.token_endpoint())
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
    Ok(RefreshedToken {
        access_token: tokens.access_token,
        expires_at: now_unix() + tokens.expires_in.unwrap_or(3600) - 60,
        new_refresh_token: tokens.refresh_token,
    })
}

/// Minimal HTML page shown in the user's browser after the redirect. `body` is
/// escaped since it can carry a provider-supplied error description.
fn page(heading: &str, body: &str) -> String {
    let esc = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    format!(
        "<html><body style=\"font-family:sans-serif;text-align:center;padding-top:20vh\">\
         <h2>{}</h2><p>{}</p></body></html>",
        esc(heading),
        esc(body),
    )
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
        let error = get("error");
        // The provider spells out what went wrong here — surface it instead of a
        // generic "cancelled", which sent us on a wild goose chase once already.
        let detail = get("error_description").unwrap_or_else(|| error.clone().unwrap_or_default());
        let body = if ok {
            page(
                "Skim is connected",
                "You can close this tab and return to the app.",
            )
        } else if let Some(err) = error.as_deref() {
            // Only a real user decline is a "cancellation"; everything else is a
            // failure worth showing the reason for.
            if err == "access_denied" {
                page("Authorization was cancelled", "You can close this tab.")
            } else {
                page("Authorization failed", &detail)
            }
        } else {
            // Favicon or stray request — keep listening.
            let _ = stream
                .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                .await;
            continue;
        };
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.shutdown().await;

        return match (get("code"), error) {
            (Some(code), _) if get("state").as_deref() == Some(expected_state) => Ok(code),
            (_, Some(err)) => {
                let code = if err == "access_denied" {
                    "oauth_cancelled"
                } else {
                    "oauth"
                };
                Err(SkimError::other(code, detail))
            }
            _ => Err(SkimError::other("oauth", "state mismatch in redirect")),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(provider: OauthProvider) -> OauthConfig {
        OauthConfig {
            provider,
            client_id: "client-123".into(),
            client_secret: String::new(),
        }
    }

    #[test]
    fn microsoft_auth_url_uses_common_authority_and_scopes() {
        let url = build_auth_url(
            &cfg(OauthProvider::Microsoft),
            "http://127.0.0.1:5000",
            "challenge",
            "state-xyz",
        );
        assert!(url
            .as_str()
            .starts_with("https://login.microsoftonline.com/common/oauth2/v2.0/authorize"));
        let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(q.get("client_id").map(String::as_str), Some("client-123"));
        assert_eq!(q.get("prompt").map(String::as_str), Some("select_account"));
        // Microsoft must NOT carry Google's offline access flag.
        assert!(!q.contains_key("access_type"));
        let scope = q.get("scope").expect("scope present");
        assert!(scope.contains("https://outlook.office.com/IMAP.AccessAsUser.All"));
        assert!(scope.contains("https://outlook.office.com/SMTP.Send"));
        assert!(scope.contains("offline_access"));
    }

    #[test]
    fn google_auth_url_keeps_offline_consent() {
        let url = build_auth_url(
            &cfg(OauthProvider::Google),
            "http://127.0.0.1:5000",
            "challenge",
            "state-xyz",
        );
        assert!(url
            .as_str()
            .starts_with("https://accounts.google.com/o/oauth2/v2/auth"));
        let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(q.get("access_type").map(String::as_str), Some("offline"));
        assert_eq!(q.get("prompt").map(String::as_str), Some("consent"));
    }

    #[test]
    fn id_token_email_prefers_preferred_username() {
        // header.payload.signature — only the payload is read.
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"preferred_username":"alice@contoso.com","email":"other@x.com"}"#);
        let jwt = format!("eyJhbGciOiJSUzI1NiJ9.{payload}.sig");
        assert_eq!(
            email_from_id_token(&jwt).as_deref(),
            Some("alice@contoso.com")
        );
    }

    #[test]
    fn id_token_email_falls_back_to_email_claim() {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"email":"bob@outlook.com"}"#);
        let jwt = format!("hdr.{payload}.sig");
        assert_eq!(
            email_from_id_token(&jwt).as_deref(),
            Some("bob@outlook.com")
        );
    }

    #[test]
    fn id_token_email_rejects_garbage() {
        assert_eq!(email_from_id_token("not-a-jwt"), None);
    }
}
