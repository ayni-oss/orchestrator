use ayni_orchestrator_runtime::activity::Stage;
use ayni_orchestrator_worker::runtime_execution::{
    ExecutionRequest, checkout, read_bounded, verify,
};
use std::{env, fs, io, path::Path, process::ExitCode};
use tracing::Instrument;

#[tokio::main]
async fn main() -> ExitCode {
    ayni_orchestrator_observability::init("runtime-worker");
    let activity = ayni_orchestrator_worker::activity::ActivityLog::new(
        (env::args().nth(1).as_deref() == Some("code"))
            .then(|| Path::new("/output/activity.json").to_path_buf()),
    );
    async {
        activity.record(ayni_orchestrator_runtime::activity::Stage::Request);
        match run(&activity).await {
            Ok(()) => {
                activity.record(ayni_orchestrator_runtime::activity::Stage::Completed);
                ExitCode::SUCCESS
            }
            Err(error) => {
                activity.fail(ayni_orchestrator_worker::activity::io_failure(&error));
                tracing::error!(error_kind = ?error.kind(), "runtime_worker_failed");
                ExitCode::FAILURE
            }
        }
    }
    .instrument(ayni_orchestrator_observability::process_span(
        "runtime-worker",
    ))
    .await
}

async fn run(activity: &ayni_orchestrator_worker::activity::ActivityLog) -> io::Result<()> {
    let operation = env::args()
        .nth(1)
        .ok_or_else(|| io::Error::other("missing operation"))?;
    if operation == "install" {
        activity.record(Stage::Install);
        fs::copy(env::current_exe()?, "/launcher/runtime-worker")?;
        return Ok(());
    }
    if operation == "assess-actions" {
        activity.record(Stage::Assessment);
        return ayni_orchestrator_worker::assessment_actions::run().await;
    }
    if operation == "prepare-output" {
        activity.record(Stage::OutputPreparation);
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
        return ayni_orchestrator_worker::coding::run(&request, Path::new("/output"), activity)
            .await
            .map_err(|error| {
                error
                    .downcast::<io::Error>()
                    .map_or_else(|_| io::Error::other("coding failed"), |error| *error)
            });
    }
    let request: ExecutionRequest =
        serde_json::from_slice(&read_bounded(Path::new("/request/execution.json"), 65_536)?)
            .map_err(io::Error::other)?;
    match operation.as_str() {
        "build" => {
            activity.record(Stage::Build);
            ayni_orchestrator_worker::runtime_execution::build(&request, Path::new("/workspace"))
        }
        "checkout" => {
            activity.record(Stage::Checkout);
            checkout(
                &request,
                Path::new("/workspace"),
                env::var("SOURCE_TOKEN").ok().as_deref(),
            )
        }
        "verify" => {
            activity.record(Stage::RuntimeVerification);
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
