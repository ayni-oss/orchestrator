//! Controlled publication endpoint and validation evidence binding.

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use ayni_orchestrator_domain::{
    ChangeConstraints, MAX_CHANGED_PATH_BYTES, MAX_CHANGED_PATHS, TaskId, repository_path_is_valid,
    scope_violations_are_bounded,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    io::Read,
    path::{Path as FilePath, PathBuf},
    process::Command,
    sync::Arc,
};
use tokio::sync::Mutex;

use crate::{
    AppState,
    development::{DevelopmentTask, PublicationAction, RepositoryBinding},
    github::GitHubApp,
    orchestration::StoreError,
};

#[derive(Clone)]
pub struct PublisherState {
    app: GitHubApp,
    checkpoint_root: PathBuf,
    evidence_root: PathBuf,
    preparation_root: PathBuf,
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
    #[serde(default)]
    change_constraints: Option<ChangeConstraints>,
    outcome: String,
    check_exit_code: i32,
    policy_edited: bool,
    tests_edited: bool,
    #[serde(default)]
    scope_violations: Vec<String>,
    #[serde(default)]
    changed_paths: Vec<String>,
    #[serde(default)]
    preparation_id: Option<String>,
    #[serde(default)]
    preparation_reused: bool,
    artifacts: Vec<Artifact>,
}

#[derive(Deserialize)]
struct Artifact {
    name: String,
    sha256: String,
}

#[derive(Serialize)]
pub struct EvidenceView {
    pub manifest_digest: String,
    pub outcome: String,
    pub check_exit_code: i32,
    pub start_revision: String,
    pub candidate_commit: String,
    pub lock_sha256: String,
    pub change_constraints: Option<ChangeConstraints>,
    pub preparation_id: Option<String>,
    pub preparation_reused: bool,
    pub policy_edited: bool,
    pub tests_edited: bool,
    pub scope_violations: Vec<String>,
    pub changed_paths: Vec<String>,
    pub artifacts: Vec<EvidenceArtifactView>,
}

#[derive(Serialize)]
pub struct EvidenceArtifactView {
    pub name: String,
    pub sha256: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidenceViewError {
    Unavailable,
    Corrupt,
}

const MAX_VALIDATION_MANIFEST_BYTES: u64 = 1_048_576;
const MAX_EVIDENCE_ARTIFACTS: usize = 16;
const MAX_EVIDENCE_ARTIFACT_NAME_BYTES: usize = 128;

fn read_manifest(path: &FilePath) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|_| "final validation manifest is absent")?;
    let mut bytes = Vec::new();
    file.take(MAX_VALIDATION_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "final validation manifest could not be read")?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_VALIDATION_MANIFEST_BYTES {
        return Err("final validation manifest exceeds the retained evidence limit".into());
    }
    Ok(bytes)
}

fn read_evidence_manifest(path: &FilePath) -> Result<Vec<u8>, EvidenceViewError> {
    let file = fs::File::open(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            EvidenceViewError::Unavailable
        } else {
            EvidenceViewError::Corrupt
        }
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_VALIDATION_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| EvidenceViewError::Corrupt)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_VALIDATION_MANIFEST_BYTES {
        return Err(EvidenceViewError::Corrupt);
    }
    Ok(bytes)
}

fn evidence_manifest_is_bounded(manifest: &Manifest) -> bool {
    matches!(
        manifest.outcome.as_str(),
        "success"
            | "quality_failure"
            | "policy_rejected"
            | "dependency_change_unsupported"
            | "scope_rejected"
    ) && manifest.changed_paths.len() <= MAX_CHANGED_PATHS
        && manifest
            .changed_paths
            .iter()
            .map(String::len)
            .sum::<usize>()
            <= MAX_CHANGED_PATH_BYTES
        && manifest
            .changed_paths
            .iter()
            .all(|path| repository_path_is_valid(path))
        && manifest.artifacts.len() <= MAX_EVIDENCE_ARTIFACTS
        && manifest.artifacts.iter().all(|artifact| {
            !artifact.name.is_empty()
                && artifact.name.len() <= MAX_EVIDENCE_ARTIFACT_NAME_BYTES
                && repository_path_is_valid(&artifact.name)
                && artifact.sha256.len() == 64
                && artifact
                    .sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
}

fn manifest_covers_change_constraints(
    manifest: &Manifest,
    constraints: &ChangeConstraints,
) -> bool {
    manifest.schema_version == 2
        && manifest.change_constraints.as_ref() == Some(constraints)
        && manifest.scope_violations.is_empty()
        && manifest.changed_paths.len() <= MAX_CHANGED_PATHS
        && manifest
            .changed_paths
            .iter()
            .map(String::len)
            .sum::<usize>()
            <= MAX_CHANGED_PATH_BYTES
        && manifest
            .changed_paths
            .iter()
            .all(|path| repository_path_is_valid(path))
        && manifest
            .changed_paths
            .iter()
            .all(|path| constraints.protecting_path(path).is_none())
        && (constraints.allow_test_edits || !manifest.tests_edited)
}

impl PublisherState {
    pub(crate) async fn publish_existing(
        &self,
        task: &crate::development::DevelopmentTask,
        bundle: &FilePath,
        branch: &str,
        title: &str,
        body: &str,
    ) -> Result<(u64, String), String> {
        self.app.publish(task, bundle, branch, title, body).await
    }

    pub(crate) fn evidence_view(
        &self,
        task: &DevelopmentTask,
    ) -> Result<EvidenceView, EvidenceViewError> {
        let bytes = read_evidence_manifest(
            &self
                .evidence_root
                .join(task.id.to_string())
                .join("manifest.json"),
        )?;
        let digest = format!("sha256:{:x}", Sha256::digest(&bytes));
        if task.validation_manifest.as_deref() != Some(&digest) {
            return Err(EvidenceViewError::Corrupt);
        }
        let manifest: Manifest =
            serde_json::from_slice(&bytes).map_err(|_| EvidenceViewError::Corrupt)?;
        if !matches!(manifest.schema_version, 1 | 2)
            || task.candidate_commit.as_deref() != Some(&manifest.candidate_commit)
            || task.input.start_revision != manifest.start_revision
            || task.repository.lock_sha256 != manifest.lock_sha256
            || (manifest.schema_version == 2
                && manifest.change_constraints.as_ref() != Some(&task.input.change_constraints))
            || !scope_violations_are_bounded(&manifest.scope_violations)
            || task.scope_violations != manifest.scope_violations
            || !evidence_manifest_is_bounded(&manifest)
        {
            return Err(EvidenceViewError::Corrupt);
        }
        if manifest.outcome == "success" {
            verify_artifacts(&self.evidence_root.join(task.id.to_string()), &manifest)
                .map_err(|_| EvidenceViewError::Corrupt)?;
        }
        Ok(EvidenceView {
            manifest_digest: digest,
            outcome: manifest.outcome,
            check_exit_code: manifest.check_exit_code,
            start_revision: manifest.start_revision,
            candidate_commit: manifest.candidate_commit,
            lock_sha256: manifest.lock_sha256,
            change_constraints: manifest.change_constraints,
            preparation_id: manifest.preparation_id,
            preparation_reused: manifest.preparation_reused,
            policy_edited: manifest.policy_edited,
            tests_edited: manifest.tests_edited,
            scope_violations: manifest.scope_violations,
            changed_paths: manifest.changed_paths,
            artifacts: manifest
                .artifacts
                .into_iter()
                .map(|artifact| EvidenceArtifactView {
                    name: artifact.name,
                    sha256: artifact.sha256,
                })
                .collect(),
        })
    }

    pub(crate) fn verify_candidate_checkpoint(
        &self,
        id: TaskId,
        candidate: &str,
    ) -> Result<(), String> {
        verify_candidate_checkpoint(&self.checkpoint_root.join(id.to_string()), candidate)
    }

    pub(crate) fn failed_validation_record(
        &self,
        id: TaskId,
        candidate: &str,
        start: &str,
        lock: &str,
        change_constraints: &ChangeConstraints,
    ) -> Option<(String, bool, String, Vec<String>)> {
        let bytes = read_manifest(
            &self
                .evidence_root
                .join(id.to_string())
                .join("manifest.json"),
        )
        .ok()?;
        let manifest: Manifest = serde_json::from_slice(&bytes).ok()?;
        if manifest.schema_version != 2
            || manifest.candidate_commit != candidate
            || manifest.start_revision != start
            || manifest.lock_sha256 != lock
            || manifest.change_constraints.as_ref() != Some(change_constraints)
            || !scope_violations_are_bounded(&manifest.scope_violations)
            || !matches!(
                manifest.outcome.as_str(),
                "quality_failure"
                    | "policy_rejected"
                    | "dependency_change_unsupported"
                    | "scope_rejected"
            )
            || (manifest.outcome == "scope_rejected" && manifest.scope_violations.is_empty())
        {
            return None;
        }
        Some((
            format!("sha256:{:x}", Sha256::digest(bytes)),
            manifest.policy_edited || manifest.tests_edited,
            manifest.outcome,
            manifest.scope_violations,
        ))
    }

    /// Mint a token for controlled Git publication.
    ///
    /// # Errors
    ///
    /// Returns an error when GitHub rejects the installation or token request.
    pub async fn installation_token(
        &self,
        repository: &RepositoryBinding,
    ) -> Result<String, String> {
        self.app.installation_token(repository).await
    }

    /// Mint a read-scoped token for checkout setup.
    ///
    /// # Errors
    ///
    /// Returns an error when GitHub rejects the installation or token request.
    pub async fn clone_token(&self, repository: &RepositoryBinding) -> Result<String, String> {
        self.app.clone_token(repository).await
    }

    #[must_use]
    pub fn checkpoint_root(&self) -> &FilePath {
        &self.checkpoint_root
    }

    #[must_use]
    pub fn evidence_root(&self) -> &FilePath {
        &self.evidence_root
    }

    #[must_use]
    pub fn preparation_root(&self) -> &FilePath {
        &self.preparation_root
    }

    /// Returns `None` when development publishing has not been configured locally.
    ///
    /// # Errors
    ///
    /// Returns an error when configuration is partial or the App key is invalid.
    pub fn from_environment() -> Result<Option<Self>, String> {
        let vars = [
            "GITHUB_APP_ID",
            "GITHUB_APP_PRIVATE_KEY_PATH",
            "DEVELOPMENT_CHECKPOINT_ROOT",
            "DEVELOPMENT_EVIDENCE_ROOT",
            "DEVELOPMENT_PREPARATION_ROOT",
            "DEVELOPMENT_PUBLISH_TOKEN",
        ];
        let present = vars.iter().filter(|name| env::var(name).is_ok()).count();
        if present == 0 {
            return Ok(None);
        }
        if present != vars.len() {
            return Err("development publisher configuration is incomplete".into());
        }
        let app_id = env::var(vars[0])
            .map_err(|_| "missing app id")?
            .parse()
            .map_err(|_| "invalid app id")?;
        let app = GitHubApp::new(
            app_id,
            FilePath::new(&env::var(vars[1]).map_err(|_| "missing app key path")?),
        )?;
        let internal_token = env::var(vars[5]).map_err(|_| "missing internal token")?;
        if internal_token.len() < 32 {
            return Err("internal publish token must be at least 32 characters".into());
        }
        Ok(Some(Self {
            app,
            checkpoint_root: PathBuf::from(
                env::var(vars[2]).map_err(|_| "missing checkpoint root")?,
            ),
            evidence_root: PathBuf::from(env::var(vars[3]).map_err(|_| "missing evidence root")?),
            preparation_root: PathBuf::from(
                env::var(vars[4]).map_err(|_| "missing preparation root")?,
            ),
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
        change_constraints: &ChangeConstraints,
    ) -> Result<(String, bool), String> {
        let directory = self.evidence_root.join(id.to_string());
        let path = directory.join("manifest.json");
        let bytes = read_manifest(&path)?;
        let manifest: Manifest =
            serde_json::from_slice(&bytes).map_err(|_| "final validation manifest is malformed")?;
        if manifest.candidate_commit != candidate
            || manifest.start_revision != start
            || manifest.outcome != "success"
            || manifest.check_exit_code != 0
            || manifest.policy_edited
            || manifest.lock_sha256 != lock
            || !manifest_covers_change_constraints(&manifest, change_constraints)
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

fn verify_candidate_checkpoint(directory: &FilePath, candidate: &str) -> Result<(), String> {
    let recorded = fs::read_to_string(directory.join("candidate-commit.txt"))
        .map_err(|_| "candidate checkpoint identity is absent")?;
    if recorded.trim() != candidate {
        return Err("candidate checkpoint identity changed after validation".into());
    }
    let bundle = directory.join("candidate.bundle");
    let output = Command::new("git")
        .args(["bundle", "list-heads"])
        .arg(&bundle)
        .output()
        .map_err(|_| "candidate checkpoint bundle could not be inspected")?;
    if !output.status.success() {
        return Err("candidate checkpoint bundle is invalid".into());
    }
    let heads = String::from_utf8(output.stdout)
        .map_err(|_| "candidate checkpoint bundle output is invalid")?;
    if !heads
        .lines()
        .any(|line| line.split_whitespace().next() == Some(candidate))
    {
        return Err("candidate checkpoint bundle does not contain the validated candidate".into());
    }
    Ok(())
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
            &task.input.change_constraints,
        )
        .map_err(PublishError::Invalid)?;
    if task.validation_manifest.as_deref() != Some(&digest) {
        return Err(PublishError::Invalid(
            "retained validation manifest changed".into(),
        ));
    }
    publisher
        .verify_candidate_checkpoint(id, candidate)
        .map_err(PublishError::Invalid)?;
    let bundle = publisher
        .checkpoint_root
        .join(id.to_string())
        .join("candidate.bundle");
    let (number, url) = publisher
        .app
        .publish(
            task,
            &bundle,
            &action.branch_name,
            &action.title,
            &action.body,
        )
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
                "publisher unavailable".to_owned(),
            ),
            Self::Invalid(message) => (StatusCode::CONFLICT, message),
            Self::Store(error) => (StatusCode::CONFLICT, error.to_string()),
            Self::External(message) => (StatusCode::BAD_GATEWAY, message),
        };
        (status, Json(serde_json::json!({"error": message}))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_fixture(directory: &FilePath) -> Result<Manifest, Box<dyn std::error::Error>> {
        let signals = br#"{"tests":"passed"}"#;
        fs::write(directory.join("signals.json"), signals)?;
        let signals_sha = format!("{:x}", Sha256::digest(signals));
        let execution = serde_json::to_vec(&serde_json::json!({
            "artifact_digest": format!("sha256:{signals_sha}")
        }))?;
        fs::write(directory.join("execution.json"), &execution)?;
        let build = br#"{"image":"fixture"}"#;
        fs::write(directory.join("build.json"), build)?;
        Ok(Manifest {
            schema_version: 2,
            start_revision: "8a555ada2f6e3ee41de2485ed13da508beaebaeb".into(),
            candidate_commit: "4e67795c0cba21bd82c21bdd6c921417fbe29ff6".into(),
            lock_sha256: "d68ca2ac621c99865bf7b50aabc668f1e9e4b85aa79389c00e13fcda4bb56fc7".into(),
            change_constraints: Some(ChangeConstraints::default()),
            outcome: "success".into(),
            check_exit_code: 0,
            policy_edited: false,
            tests_edited: false,
            scope_violations: Vec::new(),
            changed_paths: vec!["src/main.rs".into()],
            preparation_id: Some("prep-fixture".into()),
            preparation_reused: true,
            artifacts: vec![
                Artifact {
                    name: "signals.json".into(),
                    sha256: signals_sha,
                },
                Artifact {
                    name: "execution.json".into(),
                    sha256: format!("{:x}", Sha256::digest(&execution)),
                },
                Artifact {
                    name: "build.json".into(),
                    sha256: format!("{:x}", Sha256::digest(build)),
                },
            ],
        })
    }

    fn git(directory: &FilePath, args: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
        let output = Command::new("git")
            .args(args)
            .current_dir(directory)
            .output()?;
        if !output.status.success() {
            return Err(std::io::Error::other(String::from_utf8_lossy(&output.stderr)).into());
        }
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    }

    #[test]
    fn candidate_checkpoint_stays_bound_to_the_validated_commit()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        git(directory.path(), &["init", "--quiet"])?;
        git(directory.path(), &["config", "user.name", "probe"])?;
        git(
            directory.path(),
            &["config", "user.email", "probe@localhost"],
        )?;
        git(directory.path(), &["config", "commit.gpgsign", "false"])?;
        fs::write(directory.path().join("source.txt"), "first")?;
        git(directory.path(), &["add", "source.txt"])?;
        git(directory.path(), &["commit", "--quiet", "-m", "first"])?;
        let candidate = git(directory.path(), &["rev-parse", "HEAD"])?;
        git(
            directory.path(),
            &["bundle", "create", "candidate.bundle", "HEAD"],
        )?;
        fs::write(directory.path().join("candidate-commit.txt"), &candidate)?;
        verify_candidate_checkpoint(directory.path(), &candidate)?;

        fs::write(
            directory.path().join("candidate-commit.txt"),
            "0".repeat(40),
        )?;
        assert_eq!(
            verify_candidate_checkpoint(directory.path(), &candidate),
            Err("candidate checkpoint identity changed after validation".into())
        );

        fs::write(directory.path().join("candidate-commit.txt"), &candidate)?;
        fs::write(directory.path().join("source.txt"), "second")?;
        git(directory.path(), &["add", "source.txt"])?;
        git(directory.path(), &["commit", "--quiet", "-m", "second"])?;
        git(
            directory.path(),
            &["bundle", "create", "candidate.bundle", "HEAD"],
        )?;
        assert_eq!(
            verify_candidate_checkpoint(directory.path(), &candidate),
            Err("candidate checkpoint bundle does not contain the validated candidate".into())
        );
        Ok(())
    }

    #[test]
    fn successful_manifest_must_bind_and_satisfy_frozen_scope()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let mut manifest = write_fixture(directory.path())?;
        let mut constraints = ChangeConstraints::default();
        assert!(manifest_covers_change_constraints(&manifest, &constraints));

        manifest.schema_version = 1;
        assert!(!manifest_covers_change_constraints(&manifest, &constraints));
        manifest.schema_version = 2;

        manifest.tests_edited = true;
        assert!(!manifest_covers_change_constraints(&manifest, &constraints));
        constraints.allow_test_edits = true;
        manifest.change_constraints = Some(constraints.clone());
        assert!(manifest_covers_change_constraints(&manifest, &constraints));

        constraints.protected_paths = vec!["src".into()];
        manifest.change_constraints = Some(constraints.clone());
        assert!(!manifest_covers_change_constraints(&manifest, &constraints));
        Ok(())
    }

    #[test]
    fn evidence_projection_rejects_unknown_outcomes_and_unbounded_artifacts()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let mut manifest = write_fixture(directory.path())?;
        assert!(evidence_manifest_is_bounded(&manifest));
        manifest.outcome = "unexpected".to_owned();
        assert!(!evidence_manifest_is_bounded(&manifest));
        manifest.outcome = "quality_failure".to_owned();
        manifest.artifacts[0].name = "../signals.json".to_owned();
        assert!(!evidence_manifest_is_bounded(&manifest));
        manifest.artifacts[0].name = "signals.json".to_owned();
        manifest.artifacts[0].sha256 = "not-a-digest".to_owned();
        assert!(!evidence_manifest_is_bounded(&manifest));
        Ok(())
    }

    #[test]
    fn retained_manifest_reads_are_bounded() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("manifest.json");
        fs::write(
            &path,
            vec![b'x'; usize::try_from(MAX_VALIDATION_MANIFEST_BYTES)? + 1],
        )?;
        assert_eq!(
            read_manifest(&path),
            Err("final validation manifest exceeds the retained evidence limit".into())
        );
        Ok(())
    }

    #[test]
    fn retained_artifacts_must_be_present_and_unchanged() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        let manifest = write_fixture(directory.path())?;
        verify_artifacts(directory.path(), &manifest)?;

        fs::write(directory.path().join("signals.json"), b"altered")?;
        assert_eq!(
            verify_artifacts(directory.path(), &manifest),
            Err("retained validation artifact digest differs".into())
        );

        write_fixture(directory.path())?;
        fs::remove_file(directory.path().join("build.json"))?;
        assert_eq!(
            verify_artifacts(directory.path(), &manifest),
            Err("retained validation artifact is absent".into())
        );
        Ok(())
    }

    #[test]
    fn execution_artifact_must_bind_the_retained_signals() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        let mut manifest = write_fixture(directory.path())?;
        let execution = serde_json::to_vec(&serde_json::json!({
            "artifact_digest": "sha256:not-the-signals-digest"
        }))?;
        fs::write(directory.path().join("execution.json"), &execution)?;
        manifest
            .artifacts
            .iter_mut()
            .find(|artifact| artifact.name == "execution.json")
            .ok_or("fixture lacks execution artifact")?
            .sha256 = format!("{:x}", Sha256::digest(execution));
        assert_eq!(
            verify_artifacts(directory.path(), &manifest),
            Err("execution artifact does not bind signals".into())
        );
        Ok(())
    }
}
