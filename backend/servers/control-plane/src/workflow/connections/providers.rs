//! Fixed-origin, bounded credential validation. Never retain provider diagnostics.
use ayni_orchestrator_workflow_postgres::store::connections::{Connection, ProviderSettings};
use reqwest::{Client, Response};
use std::time::Duration;

pub(super) async fn validate(
    connection: &Connection,
    credential: &str,
) -> Result<Vec<String>, &'static str> {
    let client = Client::builder()
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "provider_unavailable")?;
    if connection.capability == "llm" {
        models(&client, credential).await
    } else {
        registry(&client, &connection.settings, credential).await?;
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
async fn registry(
    client: &Client,
    settings: &ProviderSettings,
    credential: &str,
) -> Result<(), &'static str> {
    let tokens: Vec<String> =
        serde_json::from_str(credential).map_err(|_| "provider_response_invalid")?;
    if tokens.len() != 2 {
        return Err("provider_response_invalid");
    }
    // Separate requests prove that neither credential silently substitutes for the other.
    for (username, password) in [
        (&settings.push_username, &tokens[0]),
        (&settings.pull_username, &tokens[1]),
    ] {
        // An unscoped registry token may carry anonymous access. Establish the
        // supplied account identity independently before accepting that response.
        let account = json(
            client
                .get("https://api.github.com/user")
                .header("User-Agent", "ayni-orchestrator")
                .bearer_auth(password)
                .send()
                .await
                .map_err(|_| "provider_unavailable")?,
        )
        .await?;
        registry_account(&account, username)?;
        let value = json(
            client
                .get("https://ghcr.io/token")
                // Authentication only: a connection has no package scope.
                // Actual push/pull permission is checked by runtime execution.
                .query(&[("service", "ghcr.io")])
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
fn registry_account(account: &serde_json::Value, username: &str) -> Result<(), &'static str> {
    if account["login"]
        .as_str()
        .is_some_and(|login| login.eq_ignore_ascii_case(username))
    {
        Ok(())
    } else {
        Err("provider_authentication_failed")
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn registry_authentication_requires_the_configured_github_account() {
        assert_eq!(
            super::registry_account(&serde_json::json!({"login":"Fixture"}), "fixture"),
            Ok(())
        );
        for account in [
            serde_json::json!({"login":"another"}),
            serde_json::json!({"token":"anonymous"}),
            serde_json::json!({}),
        ] {
            assert!(super::registry_account(&account, "fixture").is_err());
        }
    }

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
