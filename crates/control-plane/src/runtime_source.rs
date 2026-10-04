//! Bounded GitHub observations using repository-scoped installation tokens.
//! Registry credentials are intentionally absent from this client.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    env, fs,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub struct SourceClient {
    client: reqwest::Client,
    app_id: String,
    key: EncodingKey,
    setup_cursor: std::sync::atomic::AtomicUsize,
}

impl SourceClient {
    pub(crate) async fn find_pull(
        &self,
        owner: &str,
        repository: &str,
        branch: &str,
        base: &str,
        token: &str,
    ) -> Result<Option<Value>, &'static str> {
        let value = bounded(
            self.client
                .get(format!(
                    "https://api.github.com/repos/{owner}/{repository}/pulls"
                ))
                .bearer_auth(token)
                .query(&[
                    ("state", "all"),
                    ("head", &format!("{owner}:{branch}")),
                    ("base", base),
                    ("per_page", "2"),
                ]),
        )
        .await?;
        let values = value.as_array().ok_or("publication_response_invalid")?;
        match values.as_slice() {
            [] => Ok(None),
            [value] => Ok(Some(value.clone())),
            _ => Err("publication_identity_conflict"),
        }
    }

    pub(crate) async fn find_setup_pull(
        &self,
        owner: &str,
        repository: &str,
        title: &str,
        base: &str,
        token: &str,
    ) -> Result<Option<Value>, &'static str> {
        let value = bounded(
            self.client
                .get(format!(
                    "https://api.github.com/repos/{owner}/{repository}/pulls"
                ))
                .bearer_auth(token)
                .query(&[("state", "open"), ("base", base), ("per_page", "100")]),
        )
        .await?;
        let mut matches = value
            .as_array()
            .ok_or("publication_response_invalid")?
            .iter()
            .filter(|pull| {
                pull["title"].as_str() == Some(title)
                    && pull["head"]["ref"]
                        .as_str()
                        .is_some_and(|branch| branch.starts_with("setup/runtime-"))
            });
        let pull = matches.next().cloned();
        if matches.next().is_some() {
            return Err("publication_identity_conflict");
        }
        Ok(pull)
    }

    pub(crate) async fn pull(
        &self,
        owner: &str,
        repository: &str,
        number: u64,
        token: &str,
    ) -> Result<Value, &'static str> {
        bounded(
            self.client
                .get(format!(
                    "https://api.github.com/repos/{owner}/{repository}/pulls/{number}"
                ))
                .bearer_auth(token),
        )
        .await
    }

    pub(crate) async fn create_pull(
        &self,
        owner: &str,
        repository: &str,
        token: &str,
        body: &Value,
    ) -> Result<Value, &'static str> {
        bounded_operation(
            self.client
                .post(format!(
                    "https://api.github.com/repos/{owner}/{repository}/pulls"
                ))
                .bearer_auth(token)
                .json(body),
            "create_pull_request",
        )
        .await
    }

    pub(crate) fn from_environment(timeout_seconds: u64) -> Result<Self, &'static str> {
        let app_id = env::var("GITHUB_APP_ID").map_err(|_| "github_app_unconfigured")?;
        let path =
            env::var("GITHUB_APP_PRIVATE_KEY_PATH").map_err(|_| "github_app_unconfigured")?;
        let key = EncodingKey::from_rsa_pem(&fs::read(path).map_err(|_| "github_key_unavailable")?)
            .map_err(|_| "github_key_invalid")?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_seconds))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("ayni-orchestrator")
            .build()
            .map_err(|_| "github_client_unavailable")?;
        Ok(Self {
            setup_cursor: std::sync::atomic::AtomicUsize::new(0),
            client,
            app_id,
            key,
        })
    }

    pub(crate) async fn token(
        &self,
        installation: u64,
        repository_id: &str,
    ) -> Result<String, &'static str> {
        self.scoped_token(installation, repository_id, json!({"contents":"read"}))
            .await
    }

    pub(crate) async fn publication_token(
        &self,
        installation: u64,
        repository_id: &str,
    ) -> Result<String, &'static str> {
        self.scoped_token(
            installation,
            repository_id,
            json!({"contents":"write","pull_requests":"write"}),
        )
        .await
    }

    pub(crate) async fn review_token(
        &self,
        installation: u64,
        repository_id: &str,
    ) -> Result<String, &'static str> {
        self.scoped_token(
            installation,
            repository_id,
            json!({"contents":"read","actions":"read","checks":"read","pull_requests":"write"}),
        )
        .await
    }

    async fn scoped_token(
        &self,
        installation: u64,
        repository_id: &str,
        permissions: Value,
    ) -> Result<String, &'static str> {
        let jwt = self.app_jwt()?;
        let repository_id: u64 = repository_id
            .parse()
            .map_err(|_| "repository_identity_invalid")?;
        let request = self
            .client
            .post(format!(
                "https://api.github.com/app/installations/{installation}/access_tokens"
            ))
            .bearer_auth(jwt)
            .json(&json!({"repository_ids":[repository_id], "permissions":permissions}));
        let value = bounded_operation(request, "create_installation_token")
            .await
            .map_err(|reason| {
                if reason == "github_request_invalid" {
                    "github_permissions_missing"
                } else {
                    reason
                }
            })?;
        value["token"]
            .as_str()
            .filter(|v| !v.is_empty() && v.len() <= 4096)
            .map(str::to_owned)
            .ok_or("github_authentication_failed")
    }

    fn app_jwt(&self) -> Result<String, &'static str> {
        #[derive(Serialize)]
        struct Claims<'a> {
            iat: u64,
            exp: u64,
            iss: &'a str,
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "clock_invalid")?
            .as_secs();
        jsonwebtoken::encode(
            &Header::new(Algorithm::RS256),
            &Claims {
                iat: now.saturating_sub(60),
                exp: now + 540,
                iss: &self.app_id,
            },
            &self.key,
        )
        .map_err(|_| "github_authentication_failed")
    }

    pub(crate) async fn head(
        &self,
        owner: &str,
        repository: &str,
        branch: &str,
        token: &str,
    ) -> Result<String, &'static str> {
        let mut url =
            reqwest::Url::parse("https://api.github.com").map_err(|_| "github_url_invalid")?;
        url.path_segments_mut()
            .map_err(|()| "github_url_invalid")?
            .extend(["repos", owner, repository, "commits", branch]);
        let value = bounded(self.client.get(url).bearer_auth(token)).await?;
        let sha = value["sha"].as_str().ok_or("repository_head_invalid")?;
        if sha.len() != 40
            || !sha
                .bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
        {
            return Err("repository_head_invalid");
        }
        Ok(sha.to_owned())
    }

    pub(crate) async fn lock(
        &self,
        owner: &str,
        repository: &str,
        revision: &str,
        token: &str,
    ) -> Result<Vec<u8>, &'static str> {
        self.file(owner, repository, revision, ".ayni.lock", token)
            .await
    }

    pub(crate) async fn file(
        &self,
        owner: &str,
        repository: &str,
        revision: &str,
        path: &str,
        token: &str,
    ) -> Result<Vec<u8>, &'static str> {
        let mut url =
            reqwest::Url::parse("https://api.github.com").map_err(|_| "github_url_invalid")?;
        url.path_segments_mut()
            .map_err(|()| "github_url_invalid")?
            .extend(["repos", owner, repository, "contents"])
            .extend(path.split('/'));
        let value = bounded(
            self.client
                .get(url)
                .query(&[("ref", revision)])
                .bearer_auth(token),
        )
        .await?;
        if value["type"] != "file" || value["encoding"] != "base64" {
            return Err("repository_lock_invalid");
        }
        let encoded = value["content"]
            .as_str()
            .ok_or("repository_lock_invalid")?
            .replace('\n', "");
        STANDARD
            .decode(encoded)
            .map_err(|_| "repository_lock_invalid")
    }
}

include!("runtime_source_review.rs");

async fn bounded(request: reqwest::RequestBuilder) -> Result<Value, &'static str> {
    bounded_operation(request, "github_api").await
}

async fn bounded_operation(
    request: reqwest::RequestBuilder,
    operation: &'static str,
) -> Result<Value, &'static str> {
    let mut response = request
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|_| "github_unavailable")?;
    let status = response.status().as_u16();
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "github_unavailable")? {
        if bytes.len() + chunk.len() > 2_097_152 {
            return Err("github_response_oversized");
        }
        bytes.extend_from_slice(&chunk);
    }
    if !(200..=299).contains(&status) {
        let reason = github_error_reason(status, &bytes);
        tracing::warn!(
            provider = "github",
            operation,
            http_status = status,
            reason_code = reason,
            "GitHub request rejected"
        );
        return Err(reason);
    }
    serde_json::from_slice(&bytes).map_err(|_| "github_response_invalid")
}

fn github_error_reason(status: u16, body: &[u8]) -> &'static str {
    match status {
        401 | 403 => "github_access_denied",
        404 => "github_not_found",
        429 => "github_rate_limited",
        422 => {
            let value = serde_json::from_slice::<Value>(body).unwrap_or_default();
            let mut message = value["message"]
                .as_str()
                .map(str::to_lowercase)
                .unwrap_or_default();
            if let Some(errors) = value["errors"].as_array() {
                for error in errors {
                    if let Some(detail) = error["message"].as_str() {
                        message.push(' ');
                        message.push_str(&detail.to_lowercase());
                    }
                    if let Some(code) = error["code"].as_str() {
                        message.push(' ');
                        message.push_str(&code.to_lowercase());
                    }
                }
            }
            if message.contains("permissions requested are not granted") {
                "github_permissions_missing"
            } else if message.contains("reference already exists") {
                "github_reference_exists"
            } else if message.contains("reference update failed") {
                "github_reference_update_failed"
            } else if message.contains("spam") || message.contains("submitted too quickly") {
                "github_secondary_rate_limited"
            } else {
                "github_request_invalid"
            }
        }
        _ => "github_unavailable",
    }
}

include!("runtime_source_onboarding.rs");
