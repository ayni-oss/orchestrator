//! Fixed-origin, bounded credential validation. Never retain provider diagnostics.
use ayni_orchestrator_workflow_postgres::store::connections::ProviderCandidate;
use reqwest::{Client, Response};
use std::time::Duration;

pub(super) async fn validate(candidate: &ProviderCandidate) -> Result<Vec<String>, &'static str> {
    let client = Client::builder()
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "provider_unavailable")?;
    if candidate.operation.connection.capability == "llm" {
        models(&client, &candidate.credential).await
    } else {
        registry(&client, candidate).await?;
        Ok(Vec::new())
    }
}
async fn json(mut response: Response) -> Result<serde_json::Value, &'static str> {
    status(response.status().as_u16())?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "provider_unavailable")? {
        if chunk.len() > 262_144_usize.saturating_sub(bytes.len()) {
            return Err("provider_response_invalid");
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "provider_response_invalid")
}
const fn status(code: u16) -> Result<(), &'static str> {
    match code {
        200..=299 => Ok(()),
        401 => Err("provider_authentication_failed"),
        403 => Err("provider_access_denied"),
        429 => Err("provider_rate_limited"),
        500..=599 => Err("provider_unavailable"),
        _ => Err("provider_response_invalid"),
    }
}
async fn models(client: &Client, key: &str) -> Result<Vec<String>, &'static str> {
    let response = client
        .get("https://api.openai.com/v1/models")
        .bearer_auth(key)
        .send()
        .await
        .map_err(|_| "provider_unavailable")?;
    parse_models(&json(response).await?, key)
}
fn parse_models(value: &serde_json::Value, key: &str) -> Result<Vec<String>, &'static str> {
    let data = value["data"]
        .as_array()
        .ok_or("provider_response_invalid")?;
    if data.len() > 1000 {
        return Err("provider_response_invalid");
    }
    let mut models = Vec::new();
    for model in data {
        let id = model["id"].as_str().ok_or("provider_response_invalid")?;
        if id.is_empty()
            || id.len() > 200
            || id.contains(key)
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
        {
            return Err("provider_response_invalid");
        }
        models.push(id.to_owned());
    }
    models.sort();
    models.dedup();
    Ok(models)
}
async fn registry(client: &Client, candidate: &ProviderCandidate) -> Result<(), &'static str> {
    let settings = &candidate.operation.connection.settings;
    let repository = settings
        .registry_destination
        .strip_prefix("ghcr.io/")
        .ok_or("provider_response_invalid")?;
    let tokens: Vec<String> =
        serde_json::from_str(&candidate.credential).map_err(|_| "provider_response_invalid")?;
    if tokens.len() != 2 {
        return Err("provider_response_invalid");
    }
    // Separate requests prove that neither credential silently substitutes for the other.
    for (username, password, actions) in [
        (&settings.push_username, &tokens[0], "push,pull"),
        (&settings.pull_username, &tokens[1], "pull"),
    ] {
        let value = json(
            client
                .get("https://ghcr.io/token")
                .query(&[
                    ("service", "ghcr.io"),
                    (
                        "scope",
                        format!("repository:{repository}:{actions}").as_str(),
                    ),
                ])
                .basic_auth(username, Some(password))
                .send()
                .await
                .map_err(|_| "provider_unavailable")?,
        )
        .await?;
        if value["token"].as_str().is_none_or(str::is_empty) {
            return Err("provider_authentication_failed");
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn model_discovery_is_bounded_and_never_returns_credential_echoes() {
        assert_eq!(
            super::parse_models(
                &serde_json::json!({"data":[{"id":"gpt-fixture"},{"id":"gpt-fixture"}]}),
                "secret"
            ),
            Ok(vec!["gpt-fixture".into()])
        );
        for id in ["secret", "model\ntrace", ""] {
            assert!(
                super::parse_models(&serde_json::json!({"data":[{"id":id}]}), "secret").is_err()
            );
        }
        assert!(super::parse_models(&serde_json::json!({"error":"secret"}), "secret").is_err());
    }
}
