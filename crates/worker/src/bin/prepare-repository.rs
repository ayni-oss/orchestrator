//! Build and retain one immutable managed environment for compatible reuse.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tracing::{error, info};

#[derive(Deserialize)]
struct BuildRecord {
    environment_fingerprint: String,
    preparation_digest: String,
    image_tag: String,
    image_id: String,
}

#[derive(Serialize)]
struct PreparationManifest {
    schema_version: u8,
    preparation_id: String,
    revision: String,
    lock_sha256: String,
    environment_fingerprint: String,
    preparation_digest: String,
    image_tag: String,
    image_id: String,
    image_archive_sha256: String,
    build_record_sha256: String,
    duration_seconds: u64,
    completed_at: u64,
}

struct Input {
    workspace: PathBuf,
    destination: PathBuf,
    preparation_id: String,
    revision: String,
    lock_sha256: String,
}

impl Input {
    fn from_environment() -> Result<Self, io::Error> {
        Ok(Self {
            workspace: absolute("DEVELOPMENT_WORKSPACE")?,
            destination: absolute("DEVELOPMENT_PREPARATION_DIR")?,
            preparation_id: required("DEVELOPMENT_PREPARATION_ID")?,
            revision: required("DEVELOPMENT_START_REVISION")?,
            lock_sha256: required("DEVELOPMENT_LOCK_SHA256")?,
        })
    }
}

fn required(name: &str) -> Result<String, io::Error> {
    env::var(name).map_err(|_| io::Error::new(io::ErrorKind::NotFound, format!("missing {name}")))
}

fn absolute(name: &str) -> Result<PathBuf, io::Error> {
    let path = PathBuf::from(required(name)?);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} must be absolute"),
        ))
    }
}

fn command(directory: &Path, program: &str, args: &[&str]) -> Result<(), io::Error> {
    let status = Command::new(program)
        .args(args)
        .current_dir(directory)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    info!(
        program,
        exit_code = status.code(),
        "preparation command completed"
    );
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("{program} exited {status}")))
    }
}

fn output(directory: &Path, program: &str, args: &[&str]) -> Result<String, io::Error> {
    let result = Command::new(program)
        .args(args)
        .current_dir(directory)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()?;
    if !result.status.success() {
        return Err(io::Error::other(format!(
            "{program} exited {}",
            result.status
        )));
    }
    String::from_utf8(result.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "command output is not UTF-8"))
}

fn sha256(path: &Path) -> Result<String, io::Error> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn wait_for_engine(workspace: &Path) -> Result<(), io::Error> {
    for _ in 0..60 {
        if Command::new("docker")
            .arg("version")
            .current_dir(workspace)
            .output()
            .is_ok_and(|result| result.status.success())
        {
            return Ok(());
        }
        thread::sleep(Duration::from_secs(1));
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "isolated Docker engine did not become ready",
    ))
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    let input = Input::from_environment()?;
    if output(&input.workspace, "git", &["rev-parse", "HEAD"])? != input.revision {
        return Err("checkout revision differs from preparation request".into());
    }
    if !input.workspace.join(".ayni.toml").is_file()
        || !input.workspace.join(".ayni.lock").is_file()
    {
        return Err("repository requires committed .ayni.toml and .ayni.lock files".into());
    }
    if sha256(&input.workspace.join(".ayni.lock"))? != input.lock_sha256 {
        return Err("repository lock digest differs from enrollment".into());
    }
    wait_for_engine(&input.workspace)?;
    let workspace = input
        .workspace
        .to_str()
        .ok_or("workspace path is not UTF-8")?;
    command(
        &input.workspace,
        "ayni",
        &["env", "build", "--repo-root", workspace],
    )?;
    command(
        &input.workspace,
        "ayni",
        &["env", "doctor", "--repo-root", workspace],
    )?;

    let build_path = input.workspace.join(".ayni/environment/build.json");
    let build_bytes = fs::read(&build_path)?;
    let build: BuildRecord = serde_json::from_slice(&build_bytes)?;
    let actual_image = output(
        &input.workspace,
        "docker",
        &["image", "inspect", "--format", "{{.Id}}", &build.image_tag],
    )?;
    if actual_image != build.image_id {
        return Err("prepared image identity differs from Ayni build record".into());
    }

    let parent = input
        .destination
        .parent()
        .ok_or("preparation has no parent")?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".{}.tmp", input.preparation_id));
    if temporary.exists() {
        fs::remove_dir_all(&temporary)?;
    }
    fs::create_dir(&temporary)?;
    let archive = temporary.join("environment-image.tar");
    command(
        &input.workspace,
        "docker",
        &[
            "image",
            "save",
            "--output",
            archive.to_str().ok_or("archive path is not UTF-8")?,
            &build.image_tag,
        ],
    )?;
    fs::write(temporary.join("build.json"), &build_bytes)?;
    let manifest = PreparationManifest {
        schema_version: 1,
        preparation_id: input.preparation_id,
        revision: input.revision,
        lock_sha256: input.lock_sha256,
        environment_fingerprint: build.environment_fingerprint,
        preparation_digest: build.preparation_digest,
        image_tag: build.image_tag,
        image_id: build.image_id,
        image_archive_sha256: sha256(&archive)?,
        build_record_sha256: format!("{:x}", Sha256::digest(&build_bytes)),
        duration_seconds: started.elapsed().as_secs(),
        completed_at: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
    };
    fs::write(
        temporary.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    if input.destination.exists() {
        return Err("immutable preparation destination already exists".into());
    }
    fs::rename(temporary, input.destination)?;
    Ok(())
}

fn main() -> ExitCode {
    ayni_orchestrator_observability::init("prepare-repository");
    let process_span = ayni_orchestrator_observability::process_span("prepare-repository");
    let _process = process_span.enter();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            error!(
                reason_code = "repository_preparation_failed",
                error = %ayni_orchestrator_observability::redact(&error.to_string()),
                "repository preparation failed"
            );
            ExitCode::FAILURE
        }
    }
}
