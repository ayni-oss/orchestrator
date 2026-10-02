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

    pub(crate) async fn create_pull(
        &self,
        owner: &str,
        repository: &str,
        token: &str,
        body: &Value,
    ) -> Result<Value, &'static str> {
        bounded(
            self.client
                .post(format!(
                    "https://api.github.com/repos/{owner}/{repository}/pulls"
                ))
                .bearer_auth(token)
                .json(body),
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
        self.scoped_token(installation, repository_id, false).await
    }

    pub(crate) async fn publication_token(
        &self,
        installation: u64,
        repository_id: &str,
    ) -> Result<String, &'static str> {
        self.scoped_token(installation, repository_id, true).await
    }

    async fn scoped_token(
        &self,
        installation: u64,
        repository_id: &str,
        publish: bool,
    ) -> Result<String, &'static str> {
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
        let jwt = jsonwebtoken::encode(
            &Header::new(Algorithm::RS256),
            &Claims {
                iat: now.saturating_sub(60),
                exp: now + 540,
                iss: &self.app_id,
            },
            &self.key,
        )
        .map_err(|_| "github_authentication_failed")?;
        let repository_id: u64 = repository_id
            .parse()
            .map_err(|_| "repository_identity_invalid")?;
        let request = self
            .client
            .post(format!(
                "https://api.github.com/app/installations/{installation}/access_tokens"
            ))
            .bearer_auth(jwt)
            .json(&json!({"repository_ids":[repository_id], "permissions":if publish { json!({"contents":"write","pull_requests":"write"}) } else { json!({"contents":"read"}) }}));
        let value = bounded(request).await?;
        value["token"]
            .as_str()
            .filter(|v| !v.is_empty() && v.len() <= 4096)
            .map(str::to_owned)
            .ok_or("github_authentication_failed")
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
        let mut url =
            reqwest::Url::parse("https://api.github.com").map_err(|_| "github_url_invalid")?;
        url.path_segments_mut()
            .map_err(|()| "github_url_invalid")?
            .extend(["repos", owner, repository, "contents", ".ayni.lock"]);
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

async fn bounded(request: reqwest::RequestBuilder) -> Result<Value, &'static str> {
    let mut response = request
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|_| "github_unavailable")?;
    match response.status().as_u16() {
        200..=299 => {}
        401 | 403 => return Err("github_access_denied"),
        429 => return Err("github_rate_limited"),
        404 => return Err("github_not_found"),
        _ => return Err("github_unavailable"),
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "github_unavailable")? {
        if bytes.len() + chunk.len() > 2_097_152 {
            return Err("github_response_oversized");
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "github_response_invalid")
}
