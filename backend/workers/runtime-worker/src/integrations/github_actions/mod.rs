//! Trusted GitHub-hosted launcher. Never runs candidate programs on the host.
//! The Docker socket, GitHub token, and retained output are not candidate mounts.
use ayni_orchestrator_actions_contract as assessment;
use ayni_orchestrator_actions_contract::{Manifest, Outcome, RunIdentity};
use ayni_orchestrator_runtime::runtime_execution::read_bounded;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::fmt::Write as _;
use std::{collections::BTreeMap, env, fs, io, path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
};

/// Executes one trusted `workflow_run` assessment and writes bounded evidence.
/// # Errors
/// Returns a safe failure when provider, source or runtime identity cannot be established.
pub async fn run() -> io::Result<()> {
    execute().await.map_err(io::Error::other)
}

async fn prepare() -> Result<(String, Value, Manifest, std::path::PathBuf), &'static str> {
    let (repository, pull, head, base) = assessment_pull().await?;
    let run_id = variable("GITHUB_RUN_ID")?
        .parse()
        .map_err(|_| "run_invalid")?;
    let run = api(&format!("repos/{repository}/actions/runs/{run_id}"), None).await?;
    let producer_image = variable("ASSESSMENT_PRODUCER_IMAGE")?;
    let producer_digest = producer_image
        .split_once('@')
        .map(|(_, d)| d)
        .filter(|d| assessment::digest(d))
        .ok_or("producer_invalid")?;
    let identity = RunIdentity {
        repository_id: pull["base"]["repo"]["id"]
            .as_u64()
            .ok_or("repository_invalid")?
            .to_string(),
        pull_request: pull["number"].as_u64().ok_or("pull_identity_invalid")?,
        head,
        base,
        profile_revision: variable("ASSESSMENT_PROFILE_REVISION")?,
        workflow_id: run["workflow_id"].as_u64().ok_or("workflow_invalid")?,
        run_id,
        attempt: variable("GITHUB_RUN_ATTEMPT")?
            .parse()
            .map_err(|_| "attempt_invalid")?,
        workflow_sha: variable("GITHUB_SHA")?,
        producer_digest: producer_digest.into(),
    };
    let temporary = Path::new(&variable("RUNNER_TEMP")?).join(format!("assessment-{run_id}"));
    fs::create_dir(&temporary).map_err(|_| "collector_storage_unavailable")?;
    let manifest = Manifest {
        version: assessment::VERSION.into(),
        run: identity,
        runtime_digest: variable("ASSESSMENT_RUNTIME_DIGEST")?,
        lock_fingerprint: variable("ASSESSMENT_LOCK_FINGERPRINT")?,
        contract_digest: variable("ASSESSMENT_CONTRACT_DIGEST")?,
        source_before: String::new(),
        source_after: String::new(),
        ayni_version: ayni_orchestrator_profile_ayni::ayni_release::CLI_VERSION.into(),
        platform: "linux/amd64".into(),
        exit_code: -1,
        outcome: Outcome::InfrastructureFailed,
        artifacts: BTreeMap::new(),
    };
    Ok((repository, pull, manifest, temporary))
}

async fn assessment_pull() -> Result<(String, Value, String, String), &'static str> {
    let event: Value = serde_json::from_slice(
        &read_bounded(Path::new(&variable("GITHUB_EVENT_PATH")?), 1024 * 1024)
            .map_err(|_| "event_unavailable")?,
    )
    .map_err(|_| "event_invalid")?;
    let repository = variable("GITHUB_REPOSITORY")?;
    if repository.split('/').count() != 2
        || !repository
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b))
    {
        return Err("repository_invalid");
    }
    let triggering = &event["workflow_run"];
    if triggering["event"] != "pull_request" || triggering["repository"]["full_name"] != repository
    {
        return Err("trigger_invalid");
    }
    let pulls = triggering["pull_requests"]
        .as_array()
        .filter(|p| p.len() == 1)
        .ok_or("pull_identity_missing")?;
    let number = pulls[0]["number"].as_u64().ok_or("pull_identity_invalid")?;
    let pull = api(&format!("repos/{repository}/pulls/{number}"), None).await?;
    let head = string(&pull["head"], "sha")?;
    let base = string(&pull["base"], "sha")?;
    if !assessment::hex(&head, 40) || !assessment::hex(&base, 40) || pulls[0]["head"]["sha"] != head
    {
        return Err("stale_trigger");
    }
    Ok((repository, pull, head, base))
}

async fn execute() -> Result<(), &'static str> {
    let (repository, pull, mut manifest, temporary) = prepare().await?;
    let number = manifest.run.pull_request;
    let artifacts = if pull["head"]["repo"]["id"] == pull["base"]["repo"]["id"] {
        match measure(&repository, &temporary, &mut manifest).await {
            Ok(artifacts) => artifacts,
            Err(reason) => {
                eprintln!("{reason}");
                manifest.outcome = if matches!(
                    reason,
                    "policy_missing" | "lock_missing" | "lock_invalid" | "contract_changed"
                ) {
                    Outcome::PolicyRejected
                } else {
                    Outcome::InfrastructureFailed
                };
                manifest.artifacts.clear();
                BTreeMap::new()
            }
        }
    } else {
        manifest.outcome = Outcome::Unsupported;
        BTreeMap::new()
    };
    let name = format!("assessment-{}", manifest.run.run_id);
    let _ = command("docker", &["rm", "--force", &name], None, 1024).await;
    let current = api(&format!("repos/{repository}/pulls/{number}"), None).await?;
    if current["head"]["sha"] != manifest.run.head {
        manifest.outcome = Outcome::Stale;
    }
    let output = Path::new(&variable("RUNNER_TEMP")?).join("assessment-evidence");
    fs::create_dir_all(&output).map_err(|_| "evidence_unavailable")?;
    for (name, bytes) in &artifacts {
        fs::write(output.join(name), bytes).map_err(|_| "evidence_unavailable")?;
        manifest
            .artifacts
            .insert(name.clone(), assessment::sha256(bytes));
    }
    fs::write(
        output.join("manifest.json"),
        serde_json::to_vec(&manifest).map_err(|_| "manifest_invalid")?,
    )
    .map_err(|_| "evidence_unavailable")?;
    project(&repository, &manifest, &artifacts).await
}

async fn resolve_image(manifest: &Manifest) -> Result<String, &'static str> {
    let image_repository = variable("ASSESSMENT_IMAGE_REPOSITORY")?;
    if !image_repository.starts_with("ghcr.io/")
        || !image_repository
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"/._-".contains(&b))
    {
        return Err("image_invalid");
    }
    let fingerprint = manifest
        .lock_fingerprint
        .strip_prefix("sha256:")
        .filter(|v| assessment::hex(v, 64))
        .ok_or("lock_invalid")?;
    let tag = format!("{image_repository}:lock-{fingerprint}");
    command(
        "docker",
        &["pull", "--platform", "linux/amd64", &tag],
        None,
        8192,
    )
    .await?;
    let observed = command(
        "docker",
        &[
            "image",
            "inspect",
            "--format",
            "{{json .RepoDigests}}",
            &tag,
        ],
        None,
        8192,
    )
    .await?;
    let references: Vec<String> =
        serde_json::from_slice(&observed).map_err(|_| "image_digest_invalid")?;
    let image = format!("{image_repository}@{}", manifest.runtime_digest);
    if !references.contains(&image) {
        return Err("image_digest_mismatch");
    }
    Ok(image)
}

fn launch_arguments(workspace: &Path, paths: &[String], image: String, run_id: u64) -> Vec<String> {
    // A clean private scratch checkout is the only writable host mount. Read-only
    // overlays protect every tracked file and the Git metadata during execution.
    let mut args = vec![
        "run".to_owned(),
        "--name".into(),
        format!("assessment-{}", run_id),
        "--read-only".into(),
        "--network=none".into(),
        "--cap-drop=ALL".into(),
        "--security-opt=no-new-privileges".into(),
        "--user=10002:10001".into(),
        "--cpus=2".into(),
        "--memory=6g".into(),
        "--memory-swap=6g".into(),
        "--pids-limit=2048".into(),
        "--tmpfs=/scratch:rw,nosuid,nodev,size=2g,uid=10002,gid=10001".into(),
        "--tmpfs=/tmp:rw,nosuid,nodev,size=512m".into(),
        "--env=HOME=/scratch".into(),
        "--env=CARGO_HOME=/scratch/cargo".into(),
        "--env=CARGO_TARGET_DIR=/scratch/target".into(),
        "--env=MISE_CACHE_DIR=/scratch/mise".into(),
        "--workdir=/workspace".into(),
        "--mount".into(),
        format!("type=bind,source={},target=/workspace", workspace.display()),
    ];
    // Mount every source ancestor as well: a writable parent directory must
    // not be renameable around a protected file mount during execution.
    let mut directories = std::collections::BTreeSet::new();
    for path in paths {
        let mut parent = Path::new(path).parent();
        while let Some(directory) = parent {
            if directory.as_os_str().is_empty() {
                break;
            }
            directories.insert(directory.to_string_lossy().into_owned());
            parent = directory.parent();
        }
    }
    for directory in directories {
        args.extend([
            "--mount".into(),
            format!(
                "type=bind,source={}/{directory},target=/workspace/{directory}",
                workspace.display()
            ),
        ]);
    }
    for path in paths.iter().chain(std::iter::once(&".git".to_owned())) {
        args.extend([
            "--mount".into(),
            format!(
                "type=bind,source={}/{path},target=/workspace/{path},readonly",
                workspace.display()
            ),
        ]);
    }
    args.extend([
        image,
        "/usr/local/bin/ayni".into(),
        "check".into(),
        "--output".into(),
        "json".into(),
    ]);
    args
}

async fn measure(
    repository: &str,
    root: &Path,
    manifest: &mut Manifest,
) -> Result<BTreeMap<String, Vec<u8>>, &'static str> {
    let workspace = root.join("source");
    let baseline = root.join("baseline");
    checkout(repository, &manifest.run.head, &workspace).await?;
    checkout(repository, &manifest.run.workflow_sha, &baseline).await?;
    for name in [".ayni.lock", ".ayni.toml"] {
        let head =
            read_bounded(&workspace.join(name), 1024 * 1024).map_err(|_| "policy_missing")?;
        let trusted =
            read_bounded(&baseline.join(name), 1024 * 1024).map_err(|_| "policy_missing")?;
        if head != trusted {
            manifest.outcome = Outcome::Unsupported;
            return Ok(BTreeMap::new());
        }
        if name == ".ayni.toml" && assessment::sha256(&trusted) != manifest.contract_digest {
            return Err("contract_changed");
        }
    }
    let lock: Value = serde_json::from_slice(
        &read_bounded(&baseline.join(".ayni.lock"), 1024 * 1024).map_err(|_| "lock_missing")?,
    )
    .map_err(|_| "lock_invalid")?;
    if lock["fingerprint"] != manifest.lock_fingerprint
        || lock["ayni_version"] != ayni_orchestrator_profile_ayni::ayni_release::CLI_VERSION
        || lock["schema_version"] != "0.9.0"
    {
        manifest.outcome = Outcome::PolicyRejected;
        return Ok(BTreeMap::new());
    }
    let image = resolve_image(manifest).await?;
    let paths = tracked_paths(&workspace).await?;
    manifest.source_before = source_digest(&workspace, &paths).await?;
    let args = launch_arguments(&workspace, &paths, image, manifest.run.run_id);
    command(
        "chmod",
        &["-R", "a+rwX", &workspace.to_string_lossy()],
        None,
        1024,
    )
    .await?;
    let result = capture(
        "docker",
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
        None,
        assessment::MAX_BUNDLE_BYTES,
    )
    .await;
    let name = format!("assessment-{}", manifest.run.run_id);
    // Kill the entire container before touching candidate-written paths.
    let _ = command("docker", &["kill", &name], None, 1024).await;
    collect_result(root, &workspace, &paths, &name, result, manifest).await
}

async fn collect_result(
    root: &Path,
    workspace: &Path,
    paths: &[String],
    name: &str,
    result: Result<(i32, Vec<u8>), &'static str>,
    manifest: &mut Manifest,
) -> Result<BTreeMap<String, Vec<u8>>, &'static str> {
    let mut artifacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    if let Ok((exit, stdout)) = result {
        manifest.exit_code = exit;
        if !matches!(exit, 0 | 1 | 4) {
            manifest.outcome = if matches!(exit, 2 | 3) {
                Outcome::PolicyRejected
            } else {
                Outcome::InfrastructureFailed
            };
            return Ok(artifacts);
        }
        manifest.source_after = source_digest(workspace, paths).await?;
        for directory in [workspace.join(".ayni"), workspace.join(".ayni/last")] {
            if !fs::symlink_metadata(directory)
                .map_err(|_| "signals_missing")?
                .is_dir()
            {
                return Err("signals_path_unsafe");
            }
        }
        let signals = read_bounded(&workspace.join(".ayni/last/signals.json"), 8 * 1024 * 1024)
            .map_err(|_| "signals_missing")?;
        if stdout != signals || manifest.source_before != manifest.source_after {
            return Err("source_or_output_changed");
        }
        let metadata_path = root.join("runtime.json");
        command(
            "docker",
            &[
                "cp",
                &format!("{name}:/etc/ayni/runtime.json"),
                &metadata_path.to_string_lossy(),
            ],
            None,
            1024,
        )
        .await?;
        let metadata = read_bounded(&metadata_path, 1024 * 1024).map_err(|_| "runtime_missing")?;
        artifacts.insert("signals.json".into(), signals);
        artifacts.insert("runtime.json".into(), metadata);
        manifest.outcome = match exit {
            0 => Outcome::Succeeded,
            1 => Outcome::QualityFailed,
            4 => Outcome::Incomplete,
            _ => Outcome::PolicyRejected,
        };
        manifest.artifacts = artifacts
            .iter()
            .map(|(name, bytes)| (name.clone(), assessment::sha256(bytes)))
            .collect();
        let bytes = serde_json::to_vec(manifest).map_err(|_| "manifest_invalid")?;
        assessment::validate(
            &bytes,
            &artifacts,
            &assessment::Admission {
                run: &manifest.run,
                runtime_digest: &manifest.runtime_digest,
                lock_fingerprint: &manifest.lock_fingerprint,
                contract_digest: &manifest.contract_digest,
                current_head: &manifest.run.head,
                platform: &manifest.platform,
                ayni_version: &manifest.ayni_version,
            },
        )?;
    }
    Ok(artifacts)
}

fn variable(name: &str) -> Result<String, &'static str> {
    env::var(name)
        .ok()
        .filter(|v| !v.is_empty() && v.len() <= 4096 && !v.contains(['\n', '\r', '\0']))
        .ok_or("configuration_missing")
}
fn string(value: &Value, key: &str) -> Result<String, &'static str> {
    value[key]
        .as_str()
        .filter(|v| v.len() <= 4096)
        .map(str::to_owned)
        .ok_or("provider_response_invalid")
}

mod process_io;
use process_io::{api, capture, checkout, command, source_digest, tracked_paths};
mod projection;
use projection::project;

#[cfg(test)]
mod tests;
