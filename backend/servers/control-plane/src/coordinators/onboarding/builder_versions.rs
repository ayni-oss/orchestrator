//! Discover published official builders without provider or operator credentials.
use super::{AppState, OnboardingError};
use ayni_orchestrator_registry::{bounded_body, token_request};
use std::time::Duration;

impl AppState {
    pub(crate) async fn ayni_builder_versions(&self) -> Result<Vec<String>, OnboardingError> {
        tokio::time::timeout(Duration::from_secs(10), published_versions())
            .await
            .map_err(|_| OnboardingError::unavailable("builder_versions_unavailable"))?
            .map_err(|_| OnboardingError::unavailable("builder_versions_unavailable"))
    }
}

async fn published_versions() -> Result<Vec<String>, &'static str> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "builder_client_unavailable")?;
    let response = token_request(&client, "ayni-oss/ayni-builder", None, "pull")
        .send()
        .await
        .map_err(|_| "builder_versions_unavailable")?;
    if !response.status().is_success() {
        return Err("builder_versions_unavailable");
    }
    let value: serde_json::Value = serde_json::from_slice(&bounded_body(response, 16_384).await?)
        .map_err(|_| "builder_token_invalid")?;
    let token = value["token"].as_str().ok_or("builder_token_invalid")?;
    let mut versions = Vec::new();
    for contract in ayni_orchestrator_profile_ayni::compatibility::supported_contracts() {
        let response = client
            .head(format!("https://ghcr.io/v2/ayni-oss/ayni-builder/manifests/{}", contract.version))
            .bearer_auth(token)
            .header("Accept", "application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json")
            .send()
            .await
            .map_err(|_| "builder_versions_unavailable")?;
        if response.status().is_success() {
            versions.push(contract.version.to_owned());
        } else if response.status() != reqwest::StatusCode::NOT_FOUND {
            return Err("builder_versions_unavailable");
        }
    }
    Ok(versions)
}
