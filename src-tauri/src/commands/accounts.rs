use crate::db::accounts as db_accounts;
use crate::db::models::Account;
use crate::error::{Result, SkimError};
use crate::mail::{autoconfig, imap_client, oauth, sync};
use crate::secrets;
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddAccountInput {
    pub email: String,
    pub display_name: Option<String>,
    pub provider: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: String,
}

#[tauri::command]
pub fn autoconfig_lookup(email: String) -> Option<autoconfig::ServerPreset> {
    autoconfig::lookup(&email)
}

/// Whether a provider's one-click OAuth is offered, and whether its app has
/// cleared provider-side verification. `verified` is only meaningful when
/// `available`; when false, the UI shows an honest "limited sign-in" caveat.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OauthAvailability {
    pub available: bool,
    pub verified: bool,
}

fn oauth_availability(provider: oauth::OauthProvider) -> OauthAvailability {
    let available = oauth::baked_in_config(provider).is_some();
    OauthAvailability {
        available,
        verified: available && oauth::oauth_verified(provider),
    }
}

#[tauri::command]
pub fn google_oauth_available() -> OauthAvailability {
    oauth_availability(oauth::OauthProvider::Google)
}

#[tauri::command]
pub fn microsoft_oauth_available() -> OauthAvailability {
    oauth_availability(oauth::OauthProvider::Microsoft)
}

#[tauri::command]
pub async fn list_accounts(state: State<'_, AppState>) -> Result<Vec<Account>> {
    state.db.call(|conn| db_accounts::list(conn)).await
}

/// Inbox unread count per account id — fetched when the account switcher
/// opens, so the dropdown can hint where the new mail is.
#[tauri::command]
pub async fn inbox_unread_counts(
    state: State<'_, AppState>,
) -> Result<std::collections::HashMap<String, i64>> {
    let rows = state
        .db
        .call(|conn| crate::db::queries::inbox_unread_by_account(conn))
        .await?;
    Ok(rows.into_iter().collect())
}

async fn finish_add_account(
    app: &AppHandle,
    state: &State<'_, AppState>,
    account: Account,
    secret: &str,
) -> Result<Account> {
    // Refuse a mailbox that is already connected — before any secret is
    // written, so a duplicate attempt leaves the existing account untouched.
    let existing = state.db.call(|conn| db_accounts::list(conn)).await?;
    if existing
        .iter()
        .any(|a| a.email.eq_ignore_ascii_case(&account.email))
    {
        return Err(SkimError::other(
            "account_exists",
            "this account is already connected",
        ));
    }

    secrets::set(&secrets::mail_key(&account.id), secret)?;
    let acc = account.clone();
    state
        .db
        .call(move |conn| db_accounts::insert(conn, &acc))
        .await?;

    let handle = sync::spawn(
        app.clone(),
        state.db.clone(),
        account.clone(),
        state.data_dir.clone(),
    );
    state
        .engines
        .lock()
        .await
        .insert(account.id.clone(), handle);
    Ok(account)
}

/// Verify credentials against the IMAP server, then persist the account and
/// start syncing.
#[tauri::command]
pub async fn add_account(
    app: AppHandle,
    state: State<'_, AppState>,
    input: AddAccountInput,
    password: String,
) -> Result<Account> {
    let creds = imap_client::Credentials::Password(password.clone());
    let session =
        imap_client::login(&input.imap_host, input.imap_port, &input.email, &creds).await?;
    drop(session); // connection verified; the sync engine opens its own

    let account = Account {
        id: uuid::Uuid::new_v4().to_string(),
        email: input.email,
        display_name: input.display_name,
        provider: input.provider,
        imap_host: input.imap_host,
        imap_port: input.imap_port,
        smtp_host: input.smtp_host,
        smtp_port: input.smtp_port,
        smtp_security: input.smtp_security,
        auth_kind: "password".into(),
    };
    finish_add_account(&app, &state, account, &password).await
}

/// Full Google OAuth flow: browser consent → tokens → account.
#[tauri::command]
pub async fn start_google_oauth(app: AppHandle, state: State<'_, AppState>) -> Result<Account> {
    let config = oauth::baked_in_config(oauth::OauthProvider::Google).ok_or_else(|| {
        SkimError::other(
            "oauth_unconfigured",
            "Google OAuth client id is not configured",
        )
    })?;

    let opener = app.clone();
    let outcome = oauth::authorize(&config, move |url| {
        opener
            .opener()
            .open_url(url, None::<&str>)
            .map_err(|e| SkimError::other("oauth", format!("cannot open browser: {e}")))
    })
    .await?;

    // Verify the token actually works for IMAP before saving anything.
    let creds = imap_client::Credentials::OauthToken(outcome.access_token.clone());
    let session = imap_client::login("imap.gmail.com", 993, &outcome.email, &creds).await?;
    drop(session);

    let preset = autoconfig::lookup(&outcome.email);
    let account = Account {
        id: uuid::Uuid::new_v4().to_string(),
        email: outcome.email.clone(),
        display_name: None,
        provider: "gmail".into(),
        imap_host: "imap.gmail.com".into(),
        imap_port: 993,
        smtp_host: preset
            .as_ref()
            .map(|p| p.smtp_host.to_string())
            .unwrap_or_else(|| "smtp.gmail.com".into()),
        smtp_port: preset.as_ref().map(|p| p.smtp_port).unwrap_or(587),
        smtp_security: "starttls".into(),
        auth_kind: "oauth".into(),
    };
    finish_add_account(&app, &state, account, &outcome.refresh_token).await
}

/// Full Microsoft OAuth flow (Exchange Online / Office 365): browser consent →
/// tokens → account. Uses the `common` authority, so both work/school and
/// personal Microsoft accounts sign in through the same button.
#[tauri::command]
pub async fn start_microsoft_oauth(app: AppHandle, state: State<'_, AppState>) -> Result<Account> {
    let config = oauth::baked_in_config(oauth::OauthProvider::Microsoft).ok_or_else(|| {
        SkimError::other(
            "oauth_unconfigured",
            "Microsoft OAuth client id is not configured",
        )
    })?;

    let opener = app.clone();
    let outcome = oauth::authorize(&config, move |url| {
        opener
            .opener()
            .open_url(url, None::<&str>)
            .map_err(|e| SkimError::other("oauth", format!("cannot open browser: {e}")))
    })
    .await?;

    // Verify the token actually works for IMAP before saving anything.
    let creds = imap_client::Credentials::OauthToken(outcome.access_token.clone());
    let session = imap_client::login("outlook.office365.com", 993, &outcome.email, &creds).await?;
    drop(session);

    let account = Account {
        id: uuid::Uuid::new_v4().to_string(),
        email: outcome.email.clone(),
        display_name: None,
        provider: "microsoft".into(),
        imap_host: "outlook.office365.com".into(),
        imap_port: 993,
        smtp_host: "smtp.office365.com".into(),
        smtp_port: 587,
        smtp_security: "starttls".into(),
        auth_kind: "oauth".into(),
    };
    finish_add_account(&app, &state, account, &outcome.refresh_token).await
}

#[tauri::command]
pub async fn remove_account(state: State<'_, AppState>, account_id: String) -> Result<()> {
    if let Some(handle) = state.engines.lock().await.remove(&account_id) {
        handle.stop();
    }
    secrets::delete(&secrets::mail_key(&account_id))?;
    let id = account_id.clone();
    state
        .db
        .call(move |conn| db_accounts::delete(conn, &id))
        .await
}
