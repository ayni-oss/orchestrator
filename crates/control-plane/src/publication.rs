//! Controlled M3 publication endpoint and validation evidence binding.

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use ayni_orchestrator_domain::TaskId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::{Path as FilePath, PathBuf},
    sync::Arc,
};
use tokio::sync::Mutex;

use crate::{
    AppState,
    development::{DevelopmentTask, PublicationAction},
    github::GitHubApp,
    orchestration::StoreError,
};

#[derive(Clone)]
pub struct PublisherState {
    app: GitHubApp,
    checkpoint_root: PathBuf,
    evidence_root: PathBuf,
    internal_token: String,
    serial: Arc<Mutex<()>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishRequest {
    title: String,
    body: String,
}

#[derive(Serialize)]
pub struct PublishResponse {
    draft_pr_url: String,
}

#[derive(Deserialize)]
struct Manifest {
    schema_version: u8,
    start_revision: String,
    candidate_commit: String,
    lock_sha256: String,
    outcome: String,
    check_exit_code: i32,
    policy_edited: bool,
    tests_edited: bool,
    artifacts: Vec<Artifact>,
}

#[derive(Deserialize)]
struct Artifact {
    name: String,
    sha256: String,
}

impl PublisherState {
    pub(crate) async fn publish_existing(
        &self,
        task: &crate::development::DevelopmentTask,
        bundle: &FilePath,
        title: &str,
        body: &str,
    ) -> Result<(u64, String), String> {
        self.app.publish(task, bundle, title, body).await
    }

    pub(crate) fn failed_validation_record(
        &self,
        id: TaskId,
        candidate: &str,
        start: &str,
        lock: &str,
    ) -> Option<(String, bool, String)> {
        let bytes = fs::read(
            self.evidence_root
                .join(id.to_string())
                .join("manifest.json"),
        )
        .ok()?;
        let manifest: Manifest = serde_json::from_slice(&bytes).ok()?;
        if manifest.schema_version != 1
            || manifest.candidate_commit != candidate
            || manifest.start_revision != start
            || manifest.lock_sha256 != lock
            || !matches!(
                manifest.outcome.as_str(),
                "quality_failure" | "policy_rejected"
            )
        {
            return None;
        }
        Some((
            format!("sha256:{:x}", Sha256::digest(bytes)),
            manifest.policy_edited || manifest.tests_edited,
            manifest.outcome,
        ))
    }

    /// Mint a token for controlled Git publication.
    ///
    /// # Errors
    ///
    /// Returns an error when GitHub rejects the installation or token request.
    pub async fn installation_token(&self, installation_id: i64) -> Result<String, String> {
        self.app.installation_token(installation_id).await
    }

    /// Mint a read-scoped token for checkout setup.
    ///
    /// # Errors
    ///
    /// Returns an error when GitHub rejects the installation or token request.
    pub async fn clone_token(&self, installation_id: i64) -> Result<String, String> {
        self.app.clone_token(installation_id).await
    }

    #[must_use]
    pub fn checkpoint_root(&self) -> &FilePath {
        &self.checkpoint_root
    }

    #[must_use]
    pub fn evidence_root(&self) -> &FilePath {
        &self.evidence_root
    }

    /// Returns `None` when M3 publishing has not been configured locally.
    ///
    /// # Errors
    ///
    /// Returns an error when configuration is partial or the App key is invalid.
    pub fn from_environment() -> Result<Option<Self>, String> {
        let vars = [
            "M3_GITHUB_APP_ID",
            "M3_GITHUB_PRIVATE_KEY_PATH",
            "M3_CHECKPOINT_ROOT",
            "M3_EVIDENCE_ROOT",
            "M3_INTERNAL_PUBLISH_TOKEN",
        ];
        let present = vars.iter().filter(|name| env::var(name).is_ok()).count();
        if present == 0 {
            return Ok(None);
        }
        if present != vars.len() {
            return Err("M3 publisher configuration is incomplete".into());
        }
        let app_id = env::var(vars[0])
            .map_err(|_| "missing app id")?
            .parse()
            .map_err(|_| "invalid app id")?;
        let app = GitHubApp::new(
            app_id,
            FilePath::new(&env::var(vars[1]).map_err(|_| "missing app key path")?),
        )?;
        let internal_token = env::var(vars[4]).map_err(|_| "missing internal token")?;
        if internal_token.len() < 32 {
            return Err("M3 internal publish token must be at least 32 characters".into());
        }
        Ok(Some(Self {
            app,
            checkpoint_root: PathBuf::from(
                env::var(vars[2]).map_err(|_| "missing checkpoint root")?,
            ),
            evidence_root: PathBuf::from(env::var(vars[3]).map_err(|_| "missing evidence root")?),
            internal_token,
            serial: Arc::new(Mutex::new(())),
        }))
    }

    pub(crate) fn verify_manifest(
        &self,
        id: TaskId,
        candidate: &str,
        start: &str,
        lock: &str,
    ) -> Result<(String, bool), String> {
        let directory = self.evidence_root.join(id.to_string());
        let path = directory.join("manifest.json");
        let bytes = fs::read(&path).map_err(|_| "final validation manifest is absent")?;
        let manifest: Manifest =
            serde_json::from_slice(&bytes).map_err(|_| "final validation manifest is malformed")?;
        if manifest.schema_version != 1
            || manifest.candidate_commit != candidate
            || manifest.start_revision != start
            || manifest.outcome != "success"
            || manifest.check_exit_code != 0
            || manifest.policy_edited
            || manifest.lock_sha256 != lock
        {
            return Err("final validation does not cover the requested candidate".into());
        }
        verify_artifacts(&directory, &manifest)?;
        Ok((
            format!("sha256:{:x}", Sha256::digest(bytes)),
            manifest.tests_edited,
        ))
    }
}

fn verify_artifacts(directory: &FilePath, manifest: &Manifest) -> Result<(), String> {
    for name in ["signals.json", "execution.json", "build.json"] {
        let artifact = manifest
            .artifacts
            .iter()
            .find(|artifact| artifact.name == name)
            .ok_or("final validation lacks a required artifact")?;
        let actual = format!(
            "{:x}",
            Sha256::digest(
                fs::read(directory.join(name))
                    .map_err(|_| "retained validation artifact is absent")?
            )
        );
        if actual != artifact.sha256 {
            return Err("retained validation artifact digest differs".into());
        }
    }
    let execution: Value = serde_json::from_slice(
        &fs::read(directory.join("execution.json")).map_err(|_| "execution artifact is absent")?,
    )
    .map_err(|_| "execution artifact is invalid")?;
    let signals = manifest
        .artifacts
        .iter()
        .find(|artifact| artifact.name == "signals.json")
        .ok_or("signals artifact is absent")?;
    if execution.get("artifact_digest").and_then(Value::as_str)
        != Some(format!("sha256:{}", signals.sha256).as_str())
    {
        return Err("execution artifact does not bind signals".into());
    }
    Ok(())
}

pub async fn publish(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<PublishRequest>,
) -> Result<Json<PublishResponse>, PublishError> {
    let publisher = state.publisher.as_ref().ok_or(PublishError::Unavailable)?;
    let authorization = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .ok_or(PublishError::Unauthorized)?;
    if authorization != format!("Bearer {}", publisher.internal_token) {
        return Err(PublishError::Unauthorized);
    }
    if !conventional_title(&request.title)
        || request.title.len() > 120
        || request.body.trim().is_empty()
        || request.body.len() > 16_384
    {
        return Err(PublishError::Invalid("invalid PR title or body".into()));
    }
    let id =
        TaskId::parse(&task_id).map_err(|_| PublishError::Invalid("invalid task id".into()))?;
    let _guard = publisher.serial.lock().await;
    let initial = {
        let store = state.store.lock().map_err(|_| PublishError::Unavailable)?;
        store.development_task(id).map_err(PublishError::Store)?
    };
    let trusted_body = format!(
        "{}\n\n---\nValidated candidate: `{}`\nFinal evidence: `{}`\nPolicy or test edits: {}\n",
        request.body,
        initial.candidate_commit.as_deref().unwrap_or("none"),
        initial.validation_manifest.as_deref().unwrap_or("none"),
        initial.policy_or_test_edits.unwrap_or(false),
    );
    let (task, action) = {
        let store = state.store.lock().map_err(|_| PublishError::Unavailable)?;
        store
            .begin_publication(id, &request.title, &trusted_body)
            .map_err(PublishError::Store)?
    };
    if let Some(url) = action.completed_url.as_ref() {
        return Ok(Json(PublishResponse {
            draft_pr_url: url.clone(),
        }));
    }
    finish_publication(&state, publisher, id, &task, &action).await
}

async fn finish_publication(
    state: &AppState,
    publisher: &PublisherState,
    id: TaskId,
    task: &DevelopmentTask,
    action: &PublicationAction,
) -> Result<Json<PublishResponse>, PublishError> {
    let candidate = task
        .candidate_commit
        .as_deref()
        .ok_or(PublishError::Invalid("no candidate".into()))?;
    let (digest, _) = publisher
        .verify_manifest(
            id,
            candidate,
            &task.input.start_revision,
            &task.repository.lock_sha256,
        )
        .map_err(PublishError::Invalid)?;
    if task.validation_manifest.as_deref() != Some(&digest) {
        return Err(PublishError::Invalid(
            "retained validation manifest changed".into(),
        ));
    }
    let bundle = publisher
        .checkpoint_root
        .join(id.to_string())
        .join("candidate.bundle");
    let (number, url) = publisher
        .app
        .publish(task, &bundle, &action.title, &action.body)
        .await
        .map_err(PublishError::External)?;
    {
        let mut store = state.store.lock().map_err(|_| PublishError::Unavailable)?;
        store
            .complete_publication(id, candidate, number, &url)
            .map_err(PublishError::Store)?;
    }
    Ok(Json(PublishResponse { draft_pr_url: url }))
}

fn conventional_title(value: &str) -> bool {
    [
        "feat", "fix", "docs", "test", "chore", "refactor", "ci", "perf",
    ]
    .iter()
    .any(|kind| value.starts_with(&format!("{kind}(")) && value.contains("): "))
}

pub enum PublishError {
    Unauthorized,
    Unavailable,
    Invalid(String),
    Store(StoreError),
    External(String),
}

impl IntoResponse for PublishError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized".to_owned()),
            Self::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "M3 publisher unavailable".to_owned(),
            ),
            Self::Invalid(message) => (StatusCode::CONFLICT, message),
            Self::Store(error) => (StatusCode::CONFLICT, error.to_string()),
            Self::External(message) => (StatusCode::BAD_GATEWAY, message),
        };
        (status, Json(serde_json::json!({"error": message}))).into_response()
    }
}
