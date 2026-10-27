//! Authenticated internal session authority transport. Tokens are never retained in receipts.
use super::RuntimeResult;
use std::{env, io};
#[derive(serde::Deserialize)]
pub(super) struct Authority {
    #[serde(default)]
    pub(super) records: Vec<ayni_orchestrator_events::v1::ConversationContextRecord>,
    #[serde(default)]
    pub(super) next_after_ordinal: u64,
    #[serde(default)]
    pub(super) allowed: bool,
    #[serde(default)]
    pub(super) cancellation_requested: bool,
    #[serde(default)]
    pub(super) state: String,
}

pub(super) struct ProviderRuntime {
    pub(super) enabled: bool,
    pub(super) authority_url: String,
    pub(super) authority_token: String,
}

impl ProviderRuntime {
    pub(super) fn from_environment(provider: &str) -> RuntimeResult<Self> {
        let path = env::var("ORCHESTRATOR_SESSION_AUTHORITY_TOKEN_PATH")
            .map_err(|_| io::Error::other("session authority unavailable"))?;
        let authority_token = std::fs::read_to_string(path)
            .map_err(|_| io::Error::other("session authority unavailable"))?;
        let authority_url = env::var("ORCHESTRATOR_SESSION_AUTHORITY_URL")
            .map_err(|_| io::Error::other("session authority unavailable"))?;
        Ok(Self {
            enabled: env::var(if provider == "anthropic" {
                "ORCHESTRATOR_ANTHROPIC_ENABLED"
            } else {
                "ORCHESTRATOR_OPENAI_ENABLED"
            })
            .as_deref()
                == Ok("true"),
            authority_url,
            authority_token,
        })
    }
}

pub(super) async fn authority(
    http: &reqwest::Client,
    body: serde_json::Value,
    runtime: &ProviderRuntime,
) -> RuntimeResult<Authority> {
    let response = http
        .post(&runtime.authority_url)
        .bearer_auth(runtime.authority_token.trim())
        .json(&body)
        .send()
        .await?
        .error_for_status()?;
    bounded_json(response, 256 * 1024).await
}

pub(super) async fn bounded_json<T: serde::de::DeserializeOwned>(
    mut response: reqwest::Response,
    maximum: usize,
) -> RuntimeResult<T> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > maximum {
            return Err(io::Error::other("session response bound exceeded").into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(serde_json::from_slice(&bytes)?)
}
