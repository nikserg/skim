//! OpenRouter-specific pieces: key validation via its proprietary `/key`
//! endpoint and the tool-capable model catalog. The chat traffic itself goes
//! through the shared [`super::openai_compat`] client via [`endpoint`].

use super::openai_compat;
use crate::error::{Result, SkimError};
use serde::{Deserialize, Serialize};

const API_BASE: &str = "https://openrouter.ai/api/v1";

pub const DEFAULT_MODEL: &str = "anthropic/claude-sonnet-5";

/// The OpenRouter endpoint for the shared OpenAI-compatible client.
pub fn endpoint() -> openai_compat::Endpoint {
    openai_compat::Endpoint {
        base_url: API_BASE.to_string(),
        attribution: true,
    }
}

/// Validate an API key with a free key-info call.
pub async fn validate_key(key: &str) -> Result<()> {
    let resp = reqwest::Client::new()
        .get(format!("{API_BASE}/key"))
        .bearer_auth(key)
        .send()
        .await
        .map_err(|e| SkimError::other("network", e.to_string()))?;
    match resp.status().as_u16() {
        200 => Ok(()),
        401 | 403 => Err(SkimError::other("ai_key", "the API key was rejected")),
        code => Err(SkimError::other(
            "ai",
            format!("unexpected response: {code}"),
        )),
    }
}

// ---- catalog --------------------------------------------------------------

/// One model the user may pick.
#[derive(Debug, Clone, Serialize)]
pub struct Model {
    pub id: String,
    pub name: String,
}

#[derive(Deserialize)]
struct CatalogResponse {
    data: Vec<CatalogModel>,
}

#[derive(Deserialize)]
struct CatalogModel {
    id: String,
    name: String,
    #[serde(default)]
    supported_parameters: Vec<String>,
}

/// The live model catalog, narrowed to what Skim can actually drive: the chat
/// agent calls tools, so a model without tool support would silently fail on
/// the mailbox-wide assistant. Needs no API key. Ordered as OpenRouter returns
/// it — newest first.
pub async fn list_models() -> Result<Vec<Model>> {
    let resp = reqwest::Client::new()
        .get(format!("{API_BASE}/models"))
        .send()
        .await
        .map_err(|e| SkimError::other("network", e.to_string()))?;
    if resp.status() != 200 {
        return Err(SkimError::other(
            "ai",
            format!("could not load the model list: {}", resp.status()),
        ));
    }
    let catalog: CatalogResponse = resp
        .json()
        .await
        .map_err(|e| SkimError::other("ai", e.to_string()))?;
    Ok(catalog
        .data
        .into_iter()
        .filter(|m| m.supported_parameters.iter().any(|p| p == "tools"))
        .map(|m| Model {
            id: m.id,
            name: m.name,
        })
        .collect())
}
