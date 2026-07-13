//! Secrets live in the Windows Credential Manager, never in the database.

use crate::error::{Result, SkimError};

const SERVICE: &str = "Skim";

fn entry(account: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, account)
        .map_err(|e| SkimError::other("secrets", format!("credential store: {e}")))
}

pub fn set(account: &str, secret: &str) -> Result<()> {
    entry(account)?
        .set_password(secret)
        .map_err(|e| SkimError::other("secrets", format!("credential store: {e}")))
}

pub fn get(account: &str) -> Result<Option<String>> {
    match entry(account)?.get_password() {
        Ok(s) => Ok(Some(s)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(SkimError::other(
            "secrets",
            format!("credential store: {e}"),
        )),
    }
}

pub fn delete(account: &str) -> Result<()> {
    match entry(account)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(SkimError::other(
            "secrets",
            format!("credential store: {e}"),
        )),
    }
}

/// Key under which the mail credential for an account is stored. Holds the
/// password for `auth_kind = 'password'` or the OAuth refresh token for
/// `auth_kind = 'oauth'`.
pub fn mail_key(account_id: &str) -> String {
    format!("mail:{account_id}")
}

pub const ANTHROPIC_KEY: &str = "anthropic_api_key";
pub const OPENROUTER_KEY: &str = "openrouter_api_key";
