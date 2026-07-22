//! Ollama discovery: the installed, tool-capable models on a local Ollama
//! server, so the "custom" (OpenAI-compatible) provider's model field can
//! offer a picker when the configured endpoint happens to be Ollama.
//! Inference itself goes through the shared `openai_compat` client, not this
//! module — this is discovery-only.

use crate::error::{Result, SkimError};
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};

/// One installed, tool-capable model. Same shape as the OpenRouter catalog
/// entry; for Ollama both fields carry the tag (e.g. "qwen3:8b").
#[derive(Debug, Clone, Serialize)]
pub struct Model {
    pub id: String,
    pub name: String,
}

/// `GET /api/tags` — the installed models.
#[derive(Deserialize)]
struct TagsResponse {
    #[serde(default)]
    models: Vec<Tag>,
}

#[derive(Deserialize)]
struct Tag {
    name: String,
}

/// `POST /api/show` — per-model details; we only care about capabilities.
#[derive(Deserialize)]
struct ShowResponse {
    #[serde(default)]
    capabilities: Vec<String>,
}

impl ShowResponse {
    fn tools_capable(&self) -> bool {
        self.capabilities.iter().any(|c| c == "tools")
    }
}

/// The stored URL, cleaned for path joining: no surrounding space, no
/// trailing slash.
fn normalize_base(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

/// The stored custom endpoint's base URL usually ends with `/v1` (upstream's
/// `normalize_base_url` keeps the user's literal input as-is). Ollama's
/// native API lives one level up, so strip a single trailing `/v1` path
/// segment (after trimming whitespace/trailing slashes) to get the root
/// `/api/tags` and `/api/show` hang off of.
fn api_root(base_url: &str) -> String {
    let base = normalize_base(base_url);
    match base.strip_suffix("/v1") {
        Some(root) => root.to_string(),
        None => base,
    }
}

/// One `/api/show` request. A plain owned-argument async fn (rather than a
/// closure capturing references) so it composes cleanly with `stream::iter`.
async fn show_model(
    client: reqwest::Client,
    base: String,
    name: String,
) -> reqwest::Result<reqwest::Response> {
    client
        .post(format!("{base}/api/show"))
        .json(&serde_json::json!({ "model": name }))
        .send()
        .await
}

/// The installed models that can drive Skim's tool-calling features, i.e.
/// the ones we let the user pick. Also serves as the connection probe:
/// an unreachable server surfaces as `ai_unreachable`. The per-model
/// `/api/show` capability checks are independent requests, run with bounded
/// concurrency (see `SHOW_CONCURRENCY`) rather than one at a time or all at
/// once.
pub async fn list_models(base_url: &str) -> Result<Vec<Model>> {
    let base = api_root(base_url);
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{base}/api/tags"))
        .send()
        .await
        .map_err(|_| SkimError::other("ai_unreachable", "Ollama is not responding"))?;
    if resp.status() != 200 {
        return Err(SkimError::other(
            "ai",
            format!("unexpected response from Ollama: {}", resp.status()),
        ));
    }
    let tags: TagsResponse = resp
        .json()
        .await
        .map_err(|e| SkimError::other("ai", e.to_string()))?;

    // Cap in-flight `/api/show` requests: a large local catalog shouldn't
    // hammer the server with one request per model at once, and a flaky
    // server shouldn't be hit with an unbounded burst either.
    const SHOW_CONCURRENCY: usize = 8;
    let names: Vec<String> = tags.models.iter().map(|tag| tag.name.clone()).collect();
    let mut show_futures = Vec::with_capacity(names.len());
    for name in names {
        show_futures.push(show_model(client.clone(), base.clone(), name));
    }
    let shows = stream::iter(show_futures)
        .buffered(SHOW_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;

    let mut models = Vec::new();
    for (tag, resp) in tags.models.into_iter().zip(shows) {
        let resp =
            resp.map_err(|_| SkimError::other("ai_unreachable", "Ollama is not responding"))?;
        // A model that errors on /api/show just doesn't get listed.
        let Ok(show) = resp.json::<ShowResponse>().await else {
            continue;
        };
        if show.tools_capable() {
            models.push(Model {
                id: tag.name.clone(),
                name: tag.name,
            });
        }
    }
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_base_trims_space_and_trailing_slashes() {
        assert_eq!(
            normalize_base(" http://localhost:11434/ "),
            "http://localhost:11434"
        );
        assert_eq!(
            normalize_base("http://192.168.1.10:11434"),
            "http://192.168.1.10:11434"
        );
        assert_eq!(
            normalize_base("http://localhost:11434//"),
            "http://localhost:11434"
        );
    }

    #[test]
    fn api_root_strips_trailing_v1_segment() {
        assert_eq!(
            api_root("http://localhost:11434/v1"),
            "http://localhost:11434"
        );
        assert_eq!(api_root("http://localhost:11434"), "http://localhost:11434");
        assert_eq!(api_root("http://host:1/v1/"), "http://host:1");
        // A path that merely contains "v1" elsewhere isn't the `/v1` segment.
        assert_eq!(api_root("http://host/v1beta"), "http://host/v1beta");
    }

    #[test]
    fn show_response_detects_tool_capability() {
        let with: ShowResponse =
            serde_json::from_str(r#"{"license":"x","capabilities":["completion","tools"]}"#)
                .unwrap();
        assert!(with.tools_capable());
        let without: ShowResponse =
            serde_json::from_str(r#"{"capabilities":["completion","vision"]}"#).unwrap();
        assert!(!without.tools_capable());
        // Old servers may omit the field entirely.
        let missing: ShowResponse = serde_json::from_str(r#"{"license":"x"}"#).unwrap();
        assert!(!missing.tools_capable());
    }
}
