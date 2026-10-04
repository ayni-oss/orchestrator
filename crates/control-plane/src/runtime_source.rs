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
            GithubOperation::CreatePullRequest,
        )
        .await
        .map_err(|error| error.reason())
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
        let value = bounded_operation(request, GithubOperation::CreateInstallationToken)
            .await
            .map_err(|error| error.reason())?;
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
    bounded_operation(request, GithubOperation::Read)
        .await
        .map_err(|error| error.reason())
}

async fn bounded_operation(
    request: reqwest::RequestBuilder,
    operation: GithubOperation,
) -> Result<Value, GithubApiError> {
    let mut response = request
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|_| GithubApiError::unavailable())?;
    let status = response.status().as_u16();
    let request_id = response
        .headers()
        .get("x-github-request-id")
        .and_then(|value| value.to_str().ok())
        .and_then(sanitize_diagnostic);
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| GithubApiError::unavailable())?
    {
        if bytes.len() + chunk.len() > 2_097_152 {
            return Err(GithubApiError::oversized());
        }
        bytes.extend_from_slice(&chunk);
    }
    if !(200..=299).contains(&status) {
        let error = GithubApiError::from_response(status, request_id, &bytes);
        tracing::warn!(
            provider = "github",
            operation = operation.as_str(),
            http_status = status,
            reason_code = error.reason(),
            github_error_code = error.code.as_deref().unwrap_or(""),
            github_error_resource = error.resource.as_deref().unwrap_or(""),
            github_error_field = error.field.as_deref().unwrap_or(""),
            github_request_id = error.request_id.as_deref().unwrap_or(""),
            "GitHub request rejected"
        );
        return Err(error);
    }
    serde_json::from_slice(&bytes).map_err(|_| GithubApiError::invalid_response())
}

#[derive(Clone, Copy)]
enum GithubOperation {
    Read,
    CreateInstallationToken,
    CreatePullRequest,
    CreateTree,
    CreateCommit,
    CreateReference,
    ReadSetupReference,
}

impl GithubOperation {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::CreateInstallationToken => "create_installation_token",
            Self::CreatePullRequest => "create_pull_request",
            Self::CreateTree => "create_tree",
            Self::CreateCommit => "create_commit",
            Self::CreateReference => "create_reference",
            Self::ReadSetupReference => "read_setup_reference",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GithubErrorKind {
    AccessDenied,
    NotFound,
    PermissionMissing,
    RateLimited,
    SecondaryRateLimited,
    ReferenceAlreadyExists,
    ReferenceUpdateFailed,
    Conflict,
    ValidationFailed,
    Unavailable,
    ResponseOversized,
    ResponseInvalid,
}

#[derive(Debug)]
struct GithubApiError {
    kind: GithubErrorKind,
    code: Option<String>,
    resource: Option<String>,
    field: Option<String>,
    request_id: Option<String>,
}

impl GithubApiError {
    const fn unavailable() -> Self {
        Self::new(GithubErrorKind::Unavailable)
    }
    const fn oversized() -> Self {
        Self::new(GithubErrorKind::ResponseOversized)
    }
    const fn invalid_response() -> Self {
        Self::new(GithubErrorKind::ResponseInvalid)
    }

    const fn new(kind: GithubErrorKind) -> Self {
        Self {
            kind,
            code: None,
            resource: None,
            field: None,
            request_id: None,
        }
    }

    fn from_response(status: u16, request_id: Option<String>, body: &[u8]) -> Self {
        let value = serde_json::from_slice::<Value>(body).unwrap_or_default();
        let diagnostics = GithubDiagnostics::from_value(&value);
        let message = github_error_text(&value);
        let kind = match status {
            401 | 403 => GithubErrorKind::AccessDenied,
            404 => GithubErrorKind::NotFound,
            409 => GithubErrorKind::Conflict,
            429 => GithubErrorKind::RateLimited,
            422 if message.contains("permissions requested are not granted") => {
                GithubErrorKind::PermissionMissing
            }
            422 if message.contains("reference already exists") => {
                GithubErrorKind::ReferenceAlreadyExists
            }
            422 if message.contains("reference update failed") => {
                GithubErrorKind::ReferenceUpdateFailed
            }
            422 if message.contains("spam") || message.contains("submitted too quickly") => {
                GithubErrorKind::SecondaryRateLimited
            }
            400 | 422 => GithubErrorKind::ValidationFailed,
            _ => GithubErrorKind::Unavailable,
        };
        Self {
            kind,
            code: diagnostics.code,
            resource: diagnostics.resource,
            field: diagnostics.field,
            request_id: request_id.and_then(|value| sanitize_diagnostic(&value)),
        }
    }

    const fn reason(&self) -> &'static str {
        match self.kind {
            GithubErrorKind::AccessDenied => "github_access_denied",
            GithubErrorKind::NotFound => "github_not_found",
            GithubErrorKind::PermissionMissing => "github_permissions_missing",
            GithubErrorKind::RateLimited => "github_rate_limited",
            GithubErrorKind::SecondaryRateLimited => "github_secondary_rate_limited",
            GithubErrorKind::ReferenceAlreadyExists => "github_reference_exists",
            GithubErrorKind::ReferenceUpdateFailed => "github_reference_update_failed",
            GithubErrorKind::Conflict => "github_conflict",
            GithubErrorKind::ValidationFailed => "github_request_invalid",
            GithubErrorKind::Unavailable => "github_unavailable",
            GithubErrorKind::ResponseOversized => "github_response_oversized",
            GithubErrorKind::ResponseInvalid => "github_response_invalid",
        }
    }
}

#[derive(Default)]
struct GithubDiagnostics {
    code: Option<String>,
    resource: Option<String>,
    field: Option<String>,
}

impl GithubDiagnostics {
    fn from_value(value: &Value) -> Self {
        let error = value["errors"].as_array().and_then(|errors| errors.first());
        Self {
            code: error
                .and_then(|value| value["code"].as_str())
                .and_then(sanitize_diagnostic),
            resource: error
                .and_then(|value| value["resource"].as_str())
                .and_then(sanitize_diagnostic),
            field: error
                .and_then(|value| value["field"].as_str())
                .and_then(sanitize_diagnostic),
        }
    }
}

fn github_error_text(value: &Value) -> String {
    let mut message = value["message"].as_str().unwrap_or_default().to_lowercase();
    if let Some(errors) = value["errors"].as_array() {
        for error in errors {
            for key in ["message", "code"] {
                if let Some(value) = error[key].as_str() {
                    message.push(' ');
                    message.push_str(&value.to_lowercase());
                }
            }
        }
    }
    message
}

fn sanitize_diagnostic(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return None;
    }
    Some(value.to_owned())
}

include!("runtime_source_onboarding.rs");

#[cfg(test)]
mod github_error_tests {
    use super::*;

    #[test]
    fn maps_missing_setup_reference_to_not_found() {
        let error = GithubApiError::from_response(
            404,
            Some("ABC:123".to_owned()),
            br#"{"message":"Not Found"}"#,
        );
        assert_eq!(error.kind, GithubErrorKind::NotFound);
        assert_eq!(error.reason(), "github_not_found");
        assert_eq!(error.request_id.as_deref(), Some("ABC:123"));
    }

    #[test]
    fn retains_only_safe_structured_diagnostics_for_validation_errors() {
        let error = GithubApiError::from_response(
            422,
            Some("request id with spaces".to_owned()),
            br#"{"message":"Validation Failed","errors":[{"resource":"Reference","field":"ref","code":"invalid"}]}"#,
        );
        assert_eq!(error.kind, GithubErrorKind::ValidationFailed);
        assert_eq!(error.code.as_deref(), Some("invalid"));
        assert_eq!(error.resource.as_deref(), Some("Reference"));
        assert_eq!(error.field.as_deref(), Some("ref"));
        assert_eq!(error.request_id, None);
    }

    #[test]
    fn distinguishes_permission_and_secondary_rate_limit_422_responses() {
        let permission = GithubApiError::from_response(
            422,
            None,
            br#"{"message":"The permissions requested are not granted by the installation"}"#,
        );
        assert_eq!(permission.kind, GithubErrorKind::PermissionMissing);

        let rate_limited = GithubApiError::from_response(
            422,
            None,
            br#"{"message":"You have been submitted too quickly"}"#,
        );
        assert_eq!(rate_limited.kind, GithubErrorKind::SecondaryRateLimited);
    }
}
