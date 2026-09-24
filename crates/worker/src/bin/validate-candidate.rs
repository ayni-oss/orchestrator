//! Independent managed validation of a frozen M3 Git candidate bundle.

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
    thread,
    time::Duration,
};

#[derive(Serialize)]
struct Artifact {
    name: &'static str,
    sha256: String,
}

#[derive(Serialize)]
struct Manifest {
    schema_version: u8,
    start_revision: String,
    candidate_commit: String,
    lock_sha256: String,
    outcome: &'static str,
    check_exit_code: i32,
    policy_edited: bool,
    tests_edited: bool,
    changed_paths: Vec<String>,
    artifacts: Vec<Artifact>,
}

struct ValidationInput {
    candidate_bundle: PathBuf,
    candidate_commit: String,
    start_revision: String,
    expected_lock: String,
    workspace: PathBuf,
    evidence: PathBuf,
}

impl ValidationInput {
    fn from_environment() -> Result<Self, io::Error> {
        Ok(Self {
            candidate_bundle: PathBuf::from(required("DEVELOPMENT_CANDIDATE_BUNDLE")?),
            candidate_commit: required("DEVELOPMENT_CANDIDATE_COMMIT")?,
            start_revision: required("DEVELOPMENT_START_REVISION")?,
            expected_lock: required("DEVELOPMENT_LOCK_SHA256")?,
            workspace: PathBuf::from(required("DEVELOPMENT_WORKSPACE")?),
            evidence: PathBuf::from(required("DEVELOPMENT_EVIDENCE_DIR")?),
        })
    }
}

fn required(name: &str) -> Result<String, io::Error> {
    env::var(name).map_err(|_| io::Error::new(io::ErrorKind::NotFound, format!("missing {name}")))
}

fn command(directory: &Path, program: &str, args: &[&str], log: &Path) -> Result<i32, io::Error> {
    let output = Command::new(program)
        .args(args)
        .current_dir(directory)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()?;
    fs::write(log.with_extension("stdout"), &output.stdout)?;
    fs::write(log.with_extension("stderr"), &output.stderr)?;
    Ok(output.status.code().unwrap_or(1))
}

fn checked(directory: &Path, program: &str, args: &[&str], log: &Path) -> Result<(), io::Error> {
    let status = command(directory, program, args, log)?;
    if status != 0 {
        return Err(io::Error::other(format!(
            "{program} step {} exited {status}",
            log.display()
        )));
    }
    Ok(())
}

fn sha256(path: &Path) -> Result<String, io::Error> {
    Ok(format!("{:x}", Sha256::digest(fs::read(path)?)))
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let input = ValidationInput::from_environment()?;
    let (lock_digest, changed_paths) = checkout_candidate(&input)?;
    let policy_edited = changed_paths
        .iter()
        .any(|path| path == ".ayni.toml" || path == ".ayni.lock");
    let tests_edited = changed_paths
        .iter()
        .any(|path| path.contains("test") || path.contains("spec"));
    if policy_edited {
        write_manifest(
            &input.evidence,
            &Manifest {
                schema_version: 1,
                start_revision: input.start_revision,
                candidate_commit: input.candidate_commit,
                lock_sha256: lock_digest,
                outcome: "policy_rejected",
                check_exit_code: -1,
                policy_edited,
                tests_edited,
                changed_paths,
                artifacts: Vec::new(),
            },
        )?;
        return Err("candidate edited required Ayni policy or lock".into());
    }
    let exit = run_managed_check(&input.workspace, &input.evidence)?;
    retain_result(&input, lock_digest, changed_paths, tests_edited, exit)
}

fn checkout_candidate(
    input: &ValidationInput,
) -> Result<(String, Vec<String>), Box<dyn std::error::Error>> {
    let workspace = &input.workspace;
    let evidence = &input.evidence;
    fs::create_dir(workspace)?;
    fs::create_dir(evidence)?;
    checked(
        workspace,
        "git",
        &["init", "--quiet"],
        &evidence.join("git-init"),
    )?;
    checked(
        workspace,
        "git",
        &[
            "fetch",
            input
                .candidate_bundle
                .to_str()
                .ok_or("invalid bundle path")?,
            "HEAD",
        ],
        &evidence.join("git-fetch"),
    )?;
    checked(
        workspace,
        "git",
        &["checkout", "--detach", &input.candidate_commit],
        &evidence.join("git-checkout"),
    )?;
    checked(
        workspace,
        "git",
        &[
            "merge-base",
            "--is-ancestor",
            &input.start_revision,
            &input.candidate_commit,
        ],
        &evidence.join("git-ancestry"),
    )?;
    checked(
        workspace,
        "git",
        &["rev-parse", "HEAD"],
        &evidence.join("git-revision"),
    )?;
    ensure_revision(evidence, &input.candidate_commit)?;
    let lock_digest = sha256(&workspace.join(".ayni.lock"))?;
    if lock_digest != input.expected_lock {
        return Err("candidate changed the committed Ayni lock".into());
    }
    checked(
        workspace,
        "git",
        &[
            "diff",
            "--name-only",
            &input.start_revision,
            &input.candidate_commit,
        ],
        &evidence.join("git-diff"),
    )?;
    let changed_paths: Vec<String> = fs::read_to_string(evidence.join("git-diff.stdout"))?
        .lines()
        .map(str::to_owned)
        .collect();
    Ok((lock_digest, changed_paths))
}

fn ensure_revision(evidence: &Path, expected: &str) -> Result<(), Box<dyn std::error::Error>> {
    let actual = fs::read_to_string(evidence.join("git-revision.stdout"))?;
    if actual.trim() != expected {
        return Err("bundle did not produce the frozen candidate commit".into());
    }
    Ok(())
}

fn run_managed_check(workspace: &Path, evidence: &Path) -> Result<i32, Box<dyn std::error::Error>> {
    let mut engine_ready = false;
    for _ in 0..60 {
        if command(
            workspace,
            "docker",
            &["version"],
            &evidence.join("docker-ready"),
        )? == 0
        {
            engine_ready = true;
            break;
        }
        thread::sleep(Duration::from_secs(1));
    }
    if !engine_ready {
        return Err("isolated Docker engine did not become ready".into());
    }
    checked(
        workspace,
        "ayni",
        &[
            "env",
            "build",
            "--repo-root",
            workspace.to_str().ok_or("invalid workspace")?,
        ],
        &evidence.join("ayni-build"),
    )?;
    checked(
        workspace,
        "ayni",
        &[
            "env",
            "doctor",
            "--repo-root",
            workspace.to_str().ok_or("invalid workspace")?,
        ],
        &evidence.join("ayni-doctor"),
    )?;
    let config = workspace.join(".ayni.toml");
    command(
        workspace,
        "ayni",
        &[
            "check",
            "--config",
            config.to_str().ok_or("invalid config")?,
        ],
        &evidence.join("ayni-check"),
    )
    .map_err(Into::into)
}

fn retain_result(
    input: &ValidationInput,
    lock_digest: String,
    changed_paths: Vec<String>,
    tests_edited: bool,
    exit: i32,
) -> Result<(), Box<dyn std::error::Error>> {
    let root = input.workspace.join(".ayni");
    let signals = root.join("last/signals.json");
    let execution = root.join("last/execution.json");
    let build = root.join("environment/build.json");
    let signals_digest = sha256(&signals)?;
    let execution_data: Value = serde_json::from_slice(&fs::read(&execution)?)?;
    if execution_data
        .get("artifact_digest")
        .and_then(Value::as_str)
        != Some(format!("sha256:{signals_digest}").as_str())
    {
        return Err("execution artifact does not bind the signals digest".into());
    }
    let mut artifacts = Vec::new();
    for (source, name) in [
        (signals, "signals.json"),
        (execution, "execution.json"),
        (build, "build.json"),
    ] {
        let destination = input.evidence.join(name);
        fs::copy(source, &destination)?;
        artifacts.push(Artifact {
            name,
            sha256: sha256(&destination)?,
        });
    }
    let manifest = Manifest {
        schema_version: 1,
        start_revision: input.start_revision.clone(),
        candidate_commit: input.candidate_commit.clone(),
        lock_sha256: lock_digest,
        outcome: if exit == 0 {
            "success"
        } else {
            "quality_failure"
        },
        check_exit_code: exit,
        policy_edited: false,
        tests_edited,
        changed_paths,
        artifacts,
    };
    write_manifest(&input.evidence, &manifest)?;
    if exit != 0 {
        return Err(format!("Ayni quality check failed with exit {exit}").into());
    }
    Ok(())
}

fn write_manifest(evidence: &Path, manifest: &Manifest) -> Result<(), Box<dyn std::error::Error>> {
    let temporary = evidence.join("manifest.json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(manifest)?)?;
    fs::rename(temporary, evidence.join("manifest.json"))?;
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("candidate validation failed: {error}");
            ExitCode::FAILURE
        }
    }
}
