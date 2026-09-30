//! Bounded execution of a managed Ayni check.

pub mod model_usage;
pub mod runtime_contract;

use ayni_orchestrator_domain::{GitRevision, IdentityError, Sha256Digest};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
    thread,
    time::Duration,
};
use thiserror::Error;
use tracing::info;

/// GitHub repository URLs accepted by the GitHub App checkout flow.
const GITHUB_REPOSITORY_URL_PREFIX: &str = "https://github.com/";
/// The pinned Ayni binary release.
pub const SUPPORTED_AYNI_VERSION: &str = "0.13.0";

/// Immutable worker inputs supplied by the manually submitted Job.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Configuration {
    repository_url: String,
    revision: GitRevision,
    lock_digest: Sha256Digest,
    attempt_id: String,
    workspace: PathBuf,
    evidence_root: PathBuf,
}

impl Configuration {
    /// Reads fail-closed managed-check configuration from the process environment.
    ///
    /// # Errors
    ///
    /// Returns [`WorkerError`] for missing, malformed, or unsupported inputs.
    pub fn from_environment() -> Result<Self, WorkerError> {
        let repository_url = github_repository_url(required("ORCHESTRATOR_REPOSITORY_URL")?)?;
        let version = required("ORCHESTRATOR_AYNI_VERSION")?;
        if version != SUPPORTED_AYNI_VERSION {
            return Err(WorkerError::UnsupportedAyniVersion(version));
        }
        Ok(Self {
            repository_url,
            revision: GitRevision::parse(&required("ORCHESTRATOR_REVISION")?)?,
            lock_digest: Sha256Digest::parse(&required("ORCHESTRATOR_LOCK_SHA256")?)?,
            attempt_id: attempt_id(&required("ORCHESTRATOR_ATTEMPT_ID")?)?,
            workspace: absolute_path("ORCHESTRATOR_WORKSPACE")?,
            evidence_root: absolute_path("ORCHESTRATOR_EVIDENCE_ROOT")?,
        })
    }
}

fn github_repository_url(value: String) -> Result<String, WorkerError> {
    let Some(path) = value.strip_prefix(GITHUB_REPOSITORY_URL_PREFIX) else {
        return Err(WorkerError::UnsupportedRepository);
    };
    let Some(path) = path.strip_suffix(".git") else {
        return Err(WorkerError::UnsupportedRepository);
    };
    let mut components = path.split('/');
    let Some(owner) = components.next() else {
        return Err(WorkerError::UnsupportedRepository);
    };
    let Some(repository) = components.next() else {
        return Err(WorkerError::UnsupportedRepository);
    };
    if components.next().is_some() || !github_name(owner) || !github_name(repository) {
        return Err(WorkerError::UnsupportedRepository);
    }
    Ok(value)
}

fn github_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

/// Executes the managed check and retains only artifact-bound evidence.
///
/// # Errors
///
/// Returns [`WorkerError`] for source, environment, artifact, or quality failures.
pub fn run(configuration: &Configuration) -> Result<(), WorkerError> {
    let evidence = EvidenceDirectory::create(configuration)?;
    prepare_source(configuration, &evidence)?;
    prepare_environment(configuration, &evidence)?;
    let config = configuration.workspace.join(".ayni.toml");
    let status = run_command(
        &evidence,
        "ayni-check",
        &configuration.workspace,
        "ayni",
        &["check", "--config", path_text(&config)?],
    )?;
    retain_artifacts(configuration, &evidence, status)?;
    if status.success() {
        info!(revision = %configuration.revision, "managed Ayni check completed successfully");
        Ok(())
    } else {
        Err(WorkerError::QualityFailure(exit_code(status)))
    }
}

fn prepare_source(
    configuration: &Configuration,
    evidence: &EvidenceDirectory,
) -> Result<(), WorkerError> {
    fs::create_dir(&configuration.workspace).map_err(|source| WorkerError::Io {
        action: "create attempt workspace",
        source,
    })?;
    run_checked(
        evidence,
        "git-init",
        &configuration.workspace,
        "git",
        &["init", "--quiet"],
    )?;
    run_checked(
        evidence,
        "git-remote",
        &configuration.workspace,
        "git",
        &["remote", "add", "origin", &configuration.repository_url],
    )?;
    fetch_revision(configuration, evidence)?;
    run_checked(
        evidence,
        "git-checkout",
        &configuration.workspace,
        "git",
        &["checkout", "--detach", "--force", "FETCH_HEAD"],
    )?;
    let actual_revision = run_checked(
        evidence,
        "git-revision",
        &configuration.workspace,
        "git",
        &["rev-parse", "HEAD"],
    )?;
    if actual_revision.trim() != configuration.revision.as_str() {
        return Err(WorkerError::RevisionMismatch {
            expected: configuration.revision.to_string(),
            actual: actual_revision.trim().to_owned(),
        });
    }

    let lock = configuration.workspace.join(".ayni.lock");
    require_file(
        &configuration.workspace.join(".ayni.toml"),
        "Ayni configuration",
    )?;
    require_file(&lock, "Ayni lock")?;
    let actual_lock = sha256_file(&lock)?;
    if actual_lock != configuration.lock_digest.as_str() {
        return Err(WorkerError::LockMismatch {
            expected: configuration.lock_digest.to_string(),
            actual: actual_lock,
        });
    }

    Ok(())
}

fn fetch_revision(
    configuration: &Configuration,
    evidence: &EvidenceDirectory,
) -> Result<(), WorkerError> {
    let arguments = [
        "fetch",
        "--depth=1",
        "origin",
        configuration.revision.as_str(),
    ];
    let mut last_exit_code = 1;
    for attempt in 1..=3 {
        let step = format!("git-fetch-{attempt}");
        let status = run_command(evidence, &step, &configuration.workspace, "git", &arguments)?;
        if status.success() {
            return Ok(());
        }
        last_exit_code = exit_code(status);
        if attempt < 3 {
            thread::sleep(Duration::from_secs(attempt));
        }
    }
    Err(WorkerError::InfrastructureFailure {
        step: "git-fetch".to_owned(),
        exit_code: last_exit_code,
    })
}

fn prepare_environment(
    configuration: &Configuration,
    evidence: &EvidenceDirectory,
) -> Result<(), WorkerError> {
    let version = run_checked(
        evidence,
        "ayni-version",
        Path::new("/"),
        "ayni",
        &["--version"],
    )?;
    let expected_version = format!("ayni {SUPPORTED_AYNI_VERSION}");
    if version.trim() != expected_version {
        return Err(WorkerError::AyniVersionMismatch {
            expected: expected_version,
            actual: version.trim().to_owned(),
        });
    }
    let workspace = path_text(&configuration.workspace)?;
    wait_for_engine(evidence)?;
    let executor_image = build_checkout_executor(configuration, evidence)?;
    run_checked(
        evidence,
        "ayni-build",
        &configuration.workspace,
        "ayni",
        &[
            "env",
            "build",
            "--repo-root",
            workspace,
            "--executor-image",
            &executor_image,
        ],
    )?;
    run_checked(
        evidence,
        "ayni-doctor",
        &configuration.workspace,
        "ayni",
        &["env", "doctor", "--repo-root", workspace],
    )?;
    Ok(())
}

fn build_checkout_executor(
    configuration: &Configuration,
    evidence: &EvidenceDirectory,
) -> Result<String, WorkerError> {
    const REGISTRY: &str =
        "registry:2@sha256:a3d8aaa63ed8681a604f1dea0aa03f100d5895b6a58ace528858a7b332415373";
    const DEBIAN: &str = "debian:bookworm-slim@sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241";
    const MISE_VERSION: &str = "2025.2.4";
    const MISE_AMD64: &str = "c5bca644b9e8b50aa513b9f7997de72ffda086e05af4d007d0d5cb9576025fca";
    const MISE_ARM64: &str = "e3578b43b90a9031fd777d462c96a010f347f7e56f67f11b243cfc08fbff831d";
    let context = configuration.workspace.join(".orchestrator-executor");
    copy_executor_context(configuration, &context)?;
    let registry_name = format!("ayni-registry-{}", configuration.attempt_id);
    run_checked(
        evidence,
        "executor-registry",
        Path::new("/"),
        "docker",
        &[
            "run",
            "--detach",
            "--rm",
            "--name",
            &registry_name,
            "--publish",
            "127.0.0.1:5000:5000",
            REGISTRY,
        ],
    )?;
    let image = format!("127.0.0.1:5000/ayni-executor:{}", configuration.attempt_id);
    let context_text = path_text(&context)?;
    let dockerfile = context.join("ayni-env.Dockerfile");
    let dockerfile_text = path_text(&dockerfile)?;
    let target_architecture = target_architecture(env::consts::ARCH)?;
    let target_architecture_argument = format!("TARGETARCH={target_architecture}");
    run_checked(
        evidence,
        "executor-build",
        &configuration.workspace,
        "docker",
        &[
            "build",
            "--file",
            dockerfile_text,
            "--tag",
            &image,
            "--build-arg",
            &target_architecture_argument,
            "--build-arg",
            &format!("DEBIAN_IMAGE={DEBIAN}"),
            "--build-arg",
            &format!("AYNI_VERSION={SUPPORTED_AYNI_VERSION}"),
            "--build-arg",
            &format!("SOURCE_REVISION={}", configuration.revision),
            "--build-arg",
            &format!("MISE_VERSION={MISE_VERSION}"),
            "--build-arg",
            &format!("MISE_SHA256_AMD64={MISE_AMD64}"),
            "--build-arg",
            &format!("MISE_SHA256_ARM64={MISE_ARM64}"),
            context_text,
        ],
    )?;
    run_checked(
        evidence,
        "executor-push",
        Path::new("/"),
        "docker",
        &["push", &image],
    )?;
    let reference = run_checked(
        evidence,
        "executor-reference",
        Path::new("/"),
        "docker",
        &[
            "image",
            "inspect",
            "--format",
            "{{index .RepoDigests 0}}",
            &image,
        ],
    )?;
    let reference = reference.trim();
    if reference.contains("@sha256:") {
        Ok(reference.to_owned())
    } else {
        Err(WorkerError::InvalidExecutorReference(reference.to_owned()))
    }
}

fn copy_executor_context(configuration: &Configuration, context: &Path) -> Result<(), WorkerError> {
    fs::create_dir(context).map_err(|source| WorkerError::Io {
        action: "create executor context",
        source,
    })?;
    for (source, name) in [
        (
            configuration
                .workspace
                .join(".github/docker/ayni-env.Dockerfile"),
            "ayni-env.Dockerfile",
        ),
        (PathBuf::from("/usr/local/bin/ayni"), "ayni"),
        (configuration.workspace.join("LICENSE"), "LICENSE"),
        (configuration.workspace.join("NOTICE"), "NOTICE"),
    ] {
        fs::copy(source, context.join(name)).map_err(|source| WorkerError::Io {
            action: "stage checkout executor input",
            source,
        })?;
    }
    Ok(())
}

fn target_architecture(host_architecture: &str) -> Result<&'static str, WorkerError> {
    match host_architecture {
        "x86_64" => Ok("amd64"),
        "aarch64" => Ok("arm64"),
        unsupported => Err(WorkerError::UnsupportedWorkerArchitecture(
            unsupported.to_owned(),
        )),
    }
}

fn wait_for_engine(evidence: &EvidenceDirectory) -> Result<(), WorkerError> {
    for _ in 0..60 {
        if run_command(
            evidence,
            "docker-ready",
            Path::new("/"),
            "docker",
            &["version"],
        )?
        .success()
        {
            return Ok(());
        }
        thread::sleep(Duration::from_secs(1));
    }
    Err(WorkerError::InfrastructureFailure {
        step: "docker-ready".to_owned(),
        exit_code: 1,
    })
}

fn retain_artifacts(
    configuration: &Configuration,
    evidence: &EvidenceDirectory,
    status: ExitStatus,
) -> Result<(), WorkerError> {
    let signals = configuration.workspace.join(".ayni/last/signals.json");
    let execution = configuration.workspace.join(".ayni/last/execution.json");
    let build = configuration.workspace.join(".ayni/environment/build.json");
    for (path, description) in [
        (&signals, "signals artifact"),
        (&execution, "execution artifact"),
        (&build, "managed build record"),
    ] {
        require_file(path, description)?;
    }
    let signals_digest = sha256_file(&signals)?;
    validate_execution_digest(
        &fs::read(&execution).map_err(|source| WorkerError::Io {
            action: "read execution artifact",
            source,
        })?,
        &signals_digest,
    )?;
    let artifacts = [
        copy_artifact(&signals, evidence.path(), "signals.json")?,
        copy_artifact(&execution, evidence.path(), "execution.json")?,
        copy_artifact(&build, evidence.path(), "build.json")?,
    ];
    let manifest = EvidenceManifest {
        schema_version: 1,
        repository_url: &configuration.repository_url,
        revision: configuration.revision.as_str(),
        lock_sha256: configuration.lock_digest.as_str(),
        ayni_version: SUPPORTED_AYNI_VERSION,
        outcome: if status.success() {
            Outcome::Success
        } else {
            Outcome::QualityFailure
        },
        check_exit_code: exit_code(status),
        artifacts,
    };
    let content = serde_json::to_vec_pretty(&manifest).map_err(WorkerError::SerializeEvidence)?;
    fs::write(evidence.path().join("manifest.json"), content).map_err(|source| {
        WorkerError::Io {
            action: "write evidence manifest",
            source,
        }
    })?;
    Ok(())
}

fn validate_execution_digest(execution: &[u8], signals_digest: &str) -> Result<(), WorkerError> {
    let value: serde_json::Value =
        serde_json::from_slice(execution).map_err(WorkerError::InvalidExecutionArtifact)?;
    let actual = value
        .get("artifact_digest")
        .and_then(serde_json::Value::as_str)
        .ok_or(WorkerError::MissingExecutionArtifactDigest)?;
    let expected = format!("sha256:{signals_digest}");
    if actual == expected {
        Ok(())
    } else {
        Err(WorkerError::ArtifactDigestMismatch {
            expected,
            actual: actual.to_owned(),
        })
    }
}

fn run_checked(
    evidence: &EvidenceDirectory,
    step: &str,
    directory: &Path,
    program: &str,
    arguments: &[&str],
) -> Result<String, WorkerError> {
    let status = run_command(evidence, step, directory, program, arguments)?;
    if !status.success() {
        return Err(WorkerError::InfrastructureFailure {
            step: step.to_owned(),
            exit_code: exit_code(status),
        });
    }
    fs::read_to_string(evidence.path().join(format!("{step}.stdout"))).map_err(|source| {
        WorkerError::Io {
            action: "read command output",
            source,
        }
    })
}

fn run_command(
    evidence: &EvidenceDirectory,
    step: &str,
    directory: &Path,
    program: &str,
    arguments: &[&str],
) -> Result<ExitStatus, WorkerError> {
    let output = Command::new(program)
        .args(arguments)
        .env("GIT_TERMINAL_PROMPT", "0")
        .current_dir(directory)
        .output()
        .map_err(|source| WorkerError::CommandStart {
            step: step.to_owned(),
            source,
        })?;
    fs::write(
        evidence.path().join(format!("{step}.stdout")),
        output.stdout,
    )
    .map_err(|source| WorkerError::Io {
        action: "write command standard output",
        source,
    })?;
    fs::write(
        evidence.path().join(format!("{step}.stderr")),
        output.stderr,
    )
    .map_err(|source| WorkerError::Io {
        action: "write command standard error",
        source,
    })?;
    Ok(output.status)
}

fn copy_artifact(
    source: &Path,
    root: &Path,
    name: &'static str,
) -> Result<Artifact<'static>, WorkerError> {
    let destination = root.join(name);
    fs::copy(source, &destination).map_err(|source| WorkerError::Io {
        action: "copy managed artifact",
        source,
    })?;
    Ok(Artifact {
        name,
        sha256: sha256_file(&destination)?,
    })
}

fn require_file(path: &Path, description: &'static str) -> Result<(), WorkerError> {
    if path.is_file() {
        Ok(())
    } else {
        Err(WorkerError::MissingRequiredFile {
            description,
            path: path.to_path_buf(),
        })
    }
}

fn sha256_file(path: &Path) -> Result<String, WorkerError> {
    let bytes = fs::read(path).map_err(|source| WorkerError::Io {
        action: "read file for SHA-256",
        source,
    })?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn required(name: &'static str) -> Result<String, WorkerError> {
    env::var(name).map_err(|source| WorkerError::MissingEnvironment { name, source })
}
fn attempt_id(value: &str) -> Result<String, WorkerError> {
    if !value.is_empty()
        && value.bytes().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == b'-'
        })
    {
        Ok(value.to_owned())
    } else {
        Err(WorkerError::InvalidAttemptId(value.to_owned()))
    }
}
fn absolute_path(name: &'static str) -> Result<PathBuf, WorkerError> {
    let path = PathBuf::from(required(name)?);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(WorkerError::RelativePath { name, path })
    }
}
fn path_text(path: &Path) -> Result<&str, WorkerError> {
    path.to_str()
        .ok_or_else(|| WorkerError::NonUtf8Path(path.to_path_buf()))
}
fn exit_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}

struct EvidenceDirectory(PathBuf);
impl EvidenceDirectory {
    fn create(configuration: &Configuration) -> Result<Self, WorkerError> {
        let path = configuration.evidence_root.join(format!(
            "managed-check-{}-{}",
            configuration.revision, configuration.attempt_id
        ));
        fs::create_dir(&path).map_err(|source| WorkerError::CreateEvidenceDirectory {
            path: path.clone(),
            source,
        })?;
        Ok(Self(path))
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

#[derive(Serialize)]
struct EvidenceManifest<'a> {
    schema_version: u8,
    repository_url: &'a str,
    revision: &'a str,
    lock_sha256: &'a str,
    ayni_version: &'a str,
    outcome: Outcome,
    check_exit_code: i32,
    artifacts: [Artifact<'a>; 3],
}
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Success,
    QualityFailure,
}
#[derive(Serialize)]
struct Artifact<'a> {
    name: &'a str,
    sha256: String,
}

/// A worker run failed before, during, or after trustworthy evidence collection.
#[derive(Debug, Error)]
pub enum WorkerError {
    #[error("required environment variable {name} is unavailable")]
    MissingEnvironment {
        name: &'static str,
        #[source]
        source: env::VarError,
    },
    #[error("repository is not supported by the managed-check worker")]
    UnsupportedRepository,
    #[error("Ayni version {0:?} is not supported by the managed-check worker")]
    UnsupportedAyniVersion(String),
    #[error(transparent)]
    Identity(#[from] IdentityError),
    #[error("{name} must be absolute, got {path}")]
    RelativePath { name: &'static str, path: PathBuf },
    #[error("attempt ID must be a lowercase Kubernetes UID, got {0:?}")]
    InvalidAttemptId(String),
    #[error("path is not valid Unicode: {0}")]
    NonUtf8Path(PathBuf),
    #[error("failed to create evidence directory {path}")]
    CreateEvidenceDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to {action}")]
    Io {
        action: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("missing {description}: {path}")]
    MissingRequiredFile {
        description: &'static str,
        path: PathBuf,
    },
    #[error("fetched revision differs: expected {expected}, got {actual}")]
    RevisionMismatch { expected: String, actual: String },
    #[error("Ayni lock digest differs: expected {expected}, got {actual}")]
    LockMismatch { expected: String, actual: String },
    #[error("Ayni executable differs: expected {expected}, got {actual}")]
    AyniVersionMismatch { expected: String, actual: String },
    #[error("executor registry did not return an immutable digest reference: {0:?}")]
    InvalidExecutorReference(String),
    #[error("worker architecture {0:?} is not supported by the pinned executor build")]
    UnsupportedWorkerArchitecture(String),
    #[error("failed to start {step}")]
    CommandStart {
        step: String,
        #[source]
        source: io::Error,
    },
    #[error("infrastructure step {step} exited with code {exit_code}")]
    InfrastructureFailure { step: String, exit_code: i32 },
    #[error("execution artifact is not valid JSON")]
    InvalidExecutionArtifact(#[source] serde_json::Error),
    #[error("execution artifact omits artifact_digest")]
    MissingExecutionArtifactDigest,
    #[error("execution artifact digest differs: expected {expected}, got {actual}")]
    ArtifactDigestMismatch { expected: String, actual: String },
    #[error("failed to serialize evidence manifest")]
    SerializeEvidence(#[source] serde_json::Error),
    #[error("managed Ayni check completed with quality failures (exit code {0})")]
    QualityFailure(i32),
}

#[cfg(test)]
mod tests {
    use super::{
        attempt_id, github_repository_url, target_architecture, validate_execution_digest,
    };
    #[test]
    fn accepts_the_execution_binding_for_retained_signals() -> Result<(), super::WorkerError> {
        validate_execution_digest(br#"{"artifact_digest":"sha256:abc123"}"#, "abc123")
    }
    #[test]
    fn rejects_missing_or_mismatched_execution_bindings() {
        assert!(validate_execution_digest(br"{}", "abc123").is_err());
        assert!(
            validate_execution_digest(br#"{"artifact_digest":"sha256:other"}"#, "abc123").is_err()
        );
    }
    #[test]
    fn accepts_only_path_safe_kubernetes_attempt_ids() {
        assert!(matches!(
            attempt_id("a5f31d70-13b0-4d64-b7a4-cb9823a2a61d"),
            Ok(value) if value == "a5f31d70-13b0-4d64-b7a4-cb9823a2a61d"
        ));
        assert!(attempt_id("../other-attempt").is_err());
    }
    #[test]
    fn maps_worker_architectures_for_the_legacy_docker_builder() {
        assert!(matches!(target_architecture("x86_64"), Ok("amd64")));
        assert!(matches!(target_architecture("aarch64"), Ok("arm64")));
        assert!(target_architecture("riscv64").is_err());
    }
    #[test]
    fn accepts_github_repository_urls_and_rejects_credential_bearing_urls() {
        assert!(github_repository_url("https://github.com/ayni-oss/sandbox.git".into()).is_ok());
        let credential = "repository-password";
        let supplied = format!("https://user:{credential}@github.com/private/repository.git");
        let Err(error) = github_repository_url(supplied.clone()) else {
            panic!("credential-bearing repository URL was accepted");
        };
        let rendered = error.to_string();
        assert!(!rendered.contains(credential));
        assert!(!rendered.contains(&supplied));
    }
}
