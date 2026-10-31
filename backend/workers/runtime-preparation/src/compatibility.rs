//! Trusted compatibility probe. Dispatch runs before Tokio and never loads stage authority.
#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]
#[cfg(test)]
use ayni_core::TargetIdentity;
use ayni_core::{EnvironmentCertificateEnvelope, EnvironmentLock, RunArtifact, sha256_fingerprint};
use ayni_orchestrator_events::task_quality::MAX_ARTIFACT;
use ayni_orchestrator_runtime_preparation::{Failure, source};
use serde::{Deserialize, Serialize};
use std::{
    env,
    path::Path,
    process::{Command, ExitCode},
    time::{Duration, Instant},
};

const SOURCE: &str = "/inputs/source/checkout";
const WORKSPACE: &str = "/workspace";
#[derive(Serialize)]
struct Report {
    state: &'static str,
    safe_reason: &'static str,
    artifact_digest: String,
    source_digest: String,
    lock_fingerprint: String,
    certificate_digest: String,
    platform: String,
}
struct Expected {
    executable: String,
    certificate: String,
    lock: String,
    source: String,
    platform: String,
    seconds: u64,
}
struct Inputs {
    lock: EnvironmentLock,
    certificate: EnvironmentCertificateEnvelope,
    policy_digest: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Prepared {
    source_digest: String,
    tree_digest: String,
    lock_fingerprint: String,
    certificate_digest: String,
    platform: String,
}
const PREPARED: &str = "/runtime-evidence/private/prepared.json";

impl Expected {
    fn load() -> Result<Self, Failure> {
        let value = Self {
            executable: configured("ORCHESTRATOR_RUNTIME_TRUSTED_CLI_SHA256")?,
            certificate: configured("ORCHESTRATOR_RUNTIME_CERTIFICATE_DIGEST")?,
            lock: configured("ORCHESTRATOR_RUNTIME_LOCK_FINGERPRINT")?,
            source: configured("ORCHESTRATOR_RUNTIME_SOURCE_DIGEST")?,
            platform: configured("ORCHESTRATOR_RUNTIME_PLATFORM")?,
            seconds: configured("ORCHESTRATOR_RUNTIME_MAX_SECONDS")?
                .parse()
                .map_err(|_| Failure::InvalidInput)?,
        };
        if [
            &value.executable,
            &value.certificate,
            &value.lock,
            &value.source,
        ]
        .iter()
        .any(|value| !canonical_digest(value))
            || !matches!(value.platform.as_str(), "linux/amd64" | "linux/arm64")
            || !(1..=86_400).contains(&value.seconds)
        {
            return Err(Failure::InvalidInput);
        }
        Ok(value)
    }
    fn report(&self, outcome: Result<String, Failure>, success: &'static str) -> Report {
        let (state, reason, artifact) = match outcome {
            Ok(digest) => (success, "", digest),
            Err(reason) => ("invalid", reason.reason(), String::new()),
        };
        Report {
            state,
            safe_reason: reason,
            artifact_digest: artifact,
            source_digest: self.source.clone(),
            lock_fingerprint: self.lock.clone(),
            certificate_digest: self.certificate.clone(),
            platform: self.platform.clone(),
        }
    }
}
fn configured(name: &str) -> Result<String, Failure> {
    env::var(name).map_err(|_| Failure::InvalidInput)
}
fn canonical_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|v| {
        v.len() == 64
            && v.bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
    })
}

pub fn launch() -> ExitCode {
    render(probe, "managed_compatible")
}
pub fn prepare() -> ExitCode {
    render(prepare_probe, "prepared")
}
fn render(operation: fn(&Expected) -> Result<String, Failure>, success: &'static str) -> ExitCode {
    let report = match Expected::load() {
        Ok(expected) => expected.report(operation(&expected), success),
        Err(reason) => Report {
            state: "invalid",
            safe_reason: reason.reason(),
            artifact_digest: String::new(),
            source_digest: String::new(),
            lock_fingerprint: String::new(),
            certificate_digest: String::new(),
            platform: String::new(),
        },
    };
    match serde_json::to_string(&report) {
        Ok(value) => println!("{value}"),
        Err(_) => return ExitCode::from(2),
    }
    if report.state == success {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    }
}

fn inspect_inputs(expected: &Expected) -> Result<Inputs, Failure> {
    if source::snapshot_digest(Path::new(SOURCE))? != expected.source {
        return Err(Failure::SourceIdentityMismatch);
    }
    if file_digest(Path::new("/usr/local/bin/ayni"), 256 * 1024 * 1024)? != expected.executable {
        return Err(Failure::CertificationFailed);
    }
    let bytes = bounded_file(
        Path::new("/etc/ayni/runtime.json"),
        ayni_environment::MAX_CERTIFICATE_BYTES,
    )?;
    if sha256_fingerprint(&bytes) != expected.certificate {
        return Err(Failure::CertificationFailed);
    }
    let certificate: EnvironmentCertificateEnvelope =
        serde_json::from_slice(&bytes).map_err(|_| Failure::CertificationFailed)?;
    let lock: EnvironmentLock = serde_json::from_slice(&bounded_file(
        &Path::new(SOURCE).join(".ayni.lock"),
        524_288,
    )?)
    .map_err(|_| Failure::ArtifactInvalid)?;
    if lock.fingerprint() != expected.lock
        || certificate.certificate.lock_fingerprint != expected.lock
        || certificate.certificate.platform != expected.platform
        || expected.platform != native_platform()
    {
        return Err(Failure::CertificationFailed);
    }
    let policy_digest = file_digest(&Path::new(SOURCE).join(".ayni.toml"), 524_288)?;
    if policy_digest != lock.repository().contract_digest {
        return Err(Failure::SourceIdentityMismatch);
    }
    Ok(Inputs {
        lock,
        certificate,
        policy_digest,
    })
}
const fn native_platform() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "linux/arm64"
    } else if cfg!(target_arch = "x86_64") {
        "linux/amd64"
    } else {
        "unsupported"
    }
}

#[cfg(target_os = "linux")]
fn prepare_probe(expected: &Expected) -> Result<String, Failure> {
    let deadline = Instant::now() + Duration::from_secs(expected.seconds);
    verify_parent()?;
    inspect_inputs(expected)?;
    capture("copy", deadline)?;
    compare_originals(Path::new(SOURCE), Path::new(WORKSPACE))?;
    capture("prepare", deadline)?;
    compare_originals(Path::new(SOURCE), Path::new(WORKSPACE))?;
    inspect_inputs(expected)?;
    let prepared_digest = prepared_digest(Path::new(WORKSPACE))?;
    if Instant::now() >= deadline {
        return Err(Failure::ExecutionDeadline);
    }
    write_prepared(&Prepared {
        source_digest: expected.source.clone(),
        tree_digest: prepared_digest.clone(),
        lock_fingerprint: expected.lock.clone(),
        certificate_digest: expected.certificate.clone(),
        platform: expected.platform.clone(),
    })?;
    Ok(prepared_digest)
}
#[cfg(not(target_os = "linux"))]
const fn prepare_probe(_: &Expected) -> Result<String, Failure> {
    Err(Failure::CompatibilityFailed)
}

#[cfg(target_os = "linux")]
fn probe(expected: &Expected) -> Result<String, Failure> {
    let deadline = Instant::now() + Duration::from_secs(expected.seconds);
    verify_parent()?;
    let inputs = inspect_inputs(expected)?;
    let marker = read_prepared(expected)?;
    compare_originals(Path::new(SOURCE), Path::new(WORKSPACE))?;
    if prepared_digest(Path::new(WORKSPACE))? != marker.tree_digest {
        return Err(Failure::SourceIdentityMismatch);
    }
    // Kubernetes retires the entire preparation init-container cgroup before
    // this main container starts. The prepared source is mounted read-only.
    let artifact = capture("quality", deadline)?;
    compare_originals(Path::new(SOURCE), Path::new(WORKSPACE))?;
    if prepared_digest(Path::new(WORKSPACE))? != marker.tree_digest {
        return Err(Failure::SourceIdentityMismatch);
    }
    inspect_inputs(expected)?;
    let artifact_value: RunArtifact =
        serde_json::from_slice(&artifact).map_err(|_| Failure::CompatibilityFailed)?;
    let targets: Vec<_> = inputs
        .lock
        .targets()
        .iter()
        .map(|target| target.target.clone())
        .collect();
    validate_artifact(&artifact_value, expected, &inputs, &targets)?;
    if Instant::now() >= deadline {
        return Err(Failure::ExecutionDeadline);
    }
    Ok(sha256_fingerprint(&artifact))
}
#[cfg(not(target_os = "linux"))]
const fn probe(_: &Expected) -> Result<String, Failure> {
    Err(Failure::CompatibilityFailed)
}

#[cfg(target_os = "linux")]
pub fn child(stage: &str) -> ExitCode {
    let result = (|| -> Result<(), Failure> {
        use std::os::unix::process::CommandExt as _;
        if !matches!(stage, "copy" | "prepare" | "quality") {
            return Err(Failure::InvalidInput);
        }
        ayni_orchestrator_worker_sandbox::identity::isolate_child(10002, 10002)
            .map_err(|_| Failure::CompatibilityFailed)?;
        reject_inherited_sockets()?;
        ayni_orchestrator_worker_sandbox::network::isolate()
            .map_err(|_| Failure::CompatibilityFailed)?;
        ayni_orchestrator_worker_sandbox::filesystem::isolate(if stage == "copy" {
            "prepare"
        } else {
            stage
        })
        .map_err(|_| Failure::CompatibilityFailed)?;
        if stage == "copy" {
            return copy_checkout();
        }
        let mut command = if stage == "prepare" {
            let mut command = Command::new("/usr/local/bin/ayni-entrypoint");
            command.arg("/bin/true");
            command
        } else {
            environment::quality_command()?
        };
        Err({
            let _ = command.current_dir(WORKSPACE).exec();
            Failure::ExecutionFailed
        })
    })();
    if result.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    }
}
#[cfg(not(target_os = "linux"))]
pub fn child(_: &str) -> ExitCode {
    ExitCode::from(2)
}

mod artifact;
#[cfg(target_os = "linux")]
mod capture;
mod environment;
mod filesystem;
#[cfg(test)]
mod tests;
use artifact::validate_artifact;
#[cfg(target_os = "linux")]
use capture::{capture, reject_inherited_sockets, verify_parent};
pub use filesystem::install_probes;
use filesystem::{
    bounded_file, compare_originals, copy_checkout, file_digest, prepared_digest, read_prepared,
    write_prepared,
};
