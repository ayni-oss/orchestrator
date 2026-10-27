//! Fixed-origin GitHub App validation. Provider text is never exported.
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::Serialize;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(super) async fn validate(app_id: &str, pem: &str) -> Result<(), &'static str> {
    #[derive(Serialize)]
    struct Claims<'a> {
        iat: u64,
        exp: u64,
        iss: &'a str,
    }
    let key = EncodingKey::from_rsa_pem(pem.as_bytes()).map_err(|_| "github_key_invalid")?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "github_unavailable")?
        .as_secs();
    let jwt = jsonwebtoken::encode(
        &Header::new(Algorithm::RS256),
        &Claims {
            iat: now.saturating_sub(60),
            exp: now + 540,
            iss: app_id,
        },
        &key,
    )
    .map_err(|_| "github_key_invalid")?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "github_unavailable")?;
    let mut response = client
        .get("https://api.github.com/app")
        .header("User-Agent", "ayni-orchestrator")
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .bearer_auth(jwt)
        .send()
        .await
        .map_err(|_| "github_unavailable")?;
    let rate_limited = response
        .headers()
        .get("x-ratelimit-remaining")
        .is_some_and(|v| v == "0")
        || response.headers().contains_key("retry-after");
    classify_status(response.status().as_u16(), rate_limited)?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "github_unavailable")? {
        if chunk.len() > 65536_usize.saturating_sub(bytes.len()) {
            return Err("github_response_invalid");
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "github_response_invalid")?;
    let observed = value["id"]
        .as_u64()
        .ok_or("github_response_invalid")?
        .to_string();
    if observed != app_id {
        return Err("github_app_identity_mismatch");
    }
    Ok(())
}
const fn classify_status(status: u16, rate_limited: bool) -> Result<(), &'static str> {
    match status {
        200 => Ok(()),
        429 => Err("github_rate_limited"),
        403 if rate_limited => Err("github_rate_limited"),
        401 => Err("github_authentication_failed"),
        403 => Err("github_access_denied"),
        500..=599 => Err("github_unavailable"),
        _ => Err("github_response_invalid"),
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn provider_failures_keep_safe_distinct_reasons() {
        assert_eq!(
            super::classify_status(401, false),
            Err("github_authentication_failed")
        );
        assert_eq!(
            super::classify_status(403, false),
            Err("github_access_denied")
        );
        assert_eq!(
            super::classify_status(403, true),
            Err("github_rate_limited")
        );
        assert_eq!(
            super::classify_status(503, false),
            Err("github_unavailable")
        );
        assert_eq!(
            super::classify_status(302, false),
            Err("github_response_invalid")
        );
    }
    #[tokio::test]
    async fn malformed_private_key_returns_no_provider_or_secret_text() {
        assert_eq!(
            super::validate("1", "sensitive-invalid-value").await,
            Err("github_key_invalid")
        );
    }
}
