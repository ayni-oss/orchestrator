use ayni_orchestrator_worker::runtime_execution::{
    ExecutionRequest, checkout, read_bounded, verify,
};
use std::{env, fs, io, path::Path, process::ExitCode};

#[tokio::main]
async fn main() -> ExitCode {
    if matches!(run().await, Ok(())) {
        ExitCode::SUCCESS
    } else {
        // Provider, repository, and command errors may contain secrets.
        eprintln!("runtime_worker_failed");
        ExitCode::FAILURE
    }
}

async fn run() -> io::Result<()> {
    let operation = env::args()
        .nth(1)
        .ok_or_else(|| io::Error::other("missing operation"))?;
    if operation == "install" {
        fs::copy(env::current_exe()?, "/launcher/runtime-worker")?;
        return Ok(());
    }
    if operation == "assess-actions" {
        return ayni_orchestrator_worker::assessment_actions::run().await;
    }
    if operation == "prepare-output" {
        let name = env::args()
            .nth(2)
            .filter(|name| {
                name.len() <= 63
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            })
            .ok_or_else(|| io::Error::other("invalid output identity"))?;
        let path = Path::new("/outputs").join(name);
        fs::create_dir_all(&path)?;
        if !fs::symlink_metadata(&path)?.is_dir() {
            return Err(io::Error::other("unsafe output"));
        }
        return Ok(());
    }
    if operation == "code" {
        let request =
            serde_json::from_slice(&read_bounded(Path::new("/request/coding.json"), 65536)?)
                .map_err(io::Error::other)?;
        return ayni_orchestrator_worker::coding::run(&request, Path::new("/output"))
            .await
            .map_err(|_| io::Error::other("coding failed"));
    }
    let request: ExecutionRequest =
        serde_json::from_slice(&read_bounded(Path::new("/request/execution.json"), 65_536)?)
            .map_err(io::Error::other)?;
    match operation.as_str() {
        "build" => {
            ayni_orchestrator_worker::runtime_execution::build(&request, Path::new("/workspace"))
        }
        "checkout" => checkout(
            &request,
            Path::new("/workspace"),
            env::var("SOURCE_TOKEN").ok().as_deref(),
        ),
        "verify" => {
            let report = verify(&request, Path::new("/workspace"))?;
            let bytes = serde_json::to_vec(&report).map_err(io::Error::other)?;
            if bytes.len() > 4096 {
                return Err(io::Error::other("runtime report oversized"));
            }
            fs::write("/dev/termination-log", bytes)
        }
        _ => Err(io::Error::other("unsupported runtime operation")),
    }
}
