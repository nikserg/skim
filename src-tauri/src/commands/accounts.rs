use crate::db::accounts as db_accounts;
use crate::db::models::Account;
use crate::error::{Result, SkimError};
use crate::mail::{autoconfig, imap_client, oauth, sync};
use crate::secrets;
use crate::state::AppState;
use serde::Deserialize;
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

#[tauri::command]
pub async fn google_oauth_available(state: State<'_, AppState>) -> Result<bool> {
    Ok(sync::oauth_config(&state.db).await?.is_some())
}

#[tauri::command]
pub async fn list_accounts(state: State<'_, AppState>) -> Result<Vec<Account>> {
    state.db.call(|conn| db_accounts::list(conn)).await
}

async fn finish_add_account(
    app: &AppHandle,
    state: &State<'_, AppState>,
    account: Account,
    secret: &str,
) -> Result<Account> {
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
    let config = sync::oauth_config(&state.db).await?.ok_or_else(|| {
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
