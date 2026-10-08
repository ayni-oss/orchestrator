use ayni_orchestrator_profile_ayni::runtime_execution::ExecutionRequest;
use ayni_orchestrator_runtime::{activity::Stage, runtime_execution::read_bounded};
use ayni_orchestrator_worker::{operations::runtime::checkout, profiles::ayni::check};
use std::{env, fs, io, path::Path, process::ExitCode};
use tracing::Instrument;

#[tokio::main]
async fn main() -> ExitCode {
    if env::args().nth(1).as_deref() == Some("sandbox") {
        let arguments = env::args().skip(2).collect::<Vec<_>>();
        let Some((program, arguments)) = arguments.split_first() else {
            eprintln!("repository sandbox program missing");
            return ExitCode::FAILURE;
        };
        if let Err(error) =
            ayni_orchestrator_worker::operations::sandbox::execute(program, arguments)
        {
            eprintln!("repository sandbox setup failed: {error}");
        }
        return ExitCode::FAILURE;
    }
    ayni_orchestrator_observability::init("runtime-worker");
    let activity = ayni_orchestrator_worker::coding::activity::ActivityLog::new(
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
                if matches!(
                    env::args().nth(1).as_deref(),
                    Some(
                        "publish"
                            | "build"
                            | "verify"
                            | "builder-test-build"
                            | "builder-package"
                            | "builder-probe"
                            | "builder-test-verify"
                    )
                ) {
                    let message = error.to_string();
                    let fallback = match env::args().nth(1).as_deref() {
                        Some("publish") => "runtime_publication_failed",
                        Some("build" | "builder-test-build" | "builder-package") => {
                            "runtime_build_failed"
                        }
                        _ => "runtime_verification_failed",
                    };
                    let code = match message.as_str() {
                        "runtime_candidate_missing"
                        | "runtime_lock_mismatch"
                        | "runtime_platform_mismatch"
                        | "runtime_version_unsupported"
                        | "runtime_certificate_invalid"
                        | "engine_tls_or_startup_failed" => message.as_str(),
                        _ => fallback,
                    };
                    let _ = fs::write("/dev/termination-log", code);
                }
                activity.fail(ayni_orchestrator_worker::coding::activity::io_failure(
                    &error,
                ));
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

async fn run(activity: &ayni_orchestrator_worker::coding::activity::ActivityLog) -> io::Result<()> {
    let operation = env::args()
        .nth(1)
        .ok_or_else(|| io::Error::other("missing operation"))?;
    if operation == "registry-discover" {
        return ayni_orchestrator_worker::operations::image_discovery::run().await;
    }
    if operation == "install" {
        activity.record(Stage::Install);
        fs::copy(env::current_exe()?, "/launcher/runtime-worker")?;
        return Ok(());
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
            serde_json::from_slice(&read_bounded(Path::new("/request/worker.json"), 65536)?)
                .map_err(io::Error::other)?;
        return ayni_orchestrator_worker::coding::run(&request, Path::new("/output"), activity)
            .await;
    }
    if let Some(result) = trusted_image_operation(&operation).await {
        return result;
    }
    let request: ExecutionRequest =
        serde_json::from_slice(&read_bounded(Path::new("/request/execution.json"), 65_536)?)
            .map_err(io::Error::other)?;
    match operation.as_str() {
        "builder-test-verify" => {
            ayni_orchestrator_worker::profiles::ayni::builder_test::verify_fixture(
                &request,
                Path::new("/workspace"),
            )
            .await
        }
        "trust-lock" => {
            ayni_orchestrator_worker::profiles::ayni::trust_lock::run(
                &request,
                Path::new("/workspace"),
            )
            .await
        }
        "build" => {
            activity.record(Stage::Build);
            ayni_orchestrator_worker::profiles::ayni::build(&request, Path::new("/workspace")).await
        }
        "checkout" => {
            activity.record(Stage::Checkout);
            tokio::task::spawn_blocking(move || {
                checkout(
                    &request,
                    Path::new("/workspace"),
                    env::var("SOURCE_TOKEN").ok().as_deref(),
                )
            })
            .await
            .map_err(io::Error::other)?
        }
        "verify" => {
            activity.record(Stage::RuntimeVerification);
            let report = check(&request, Path::new("/workspace")).await?;
            let bytes = serde_json::to_vec(&report).map_err(io::Error::other)?;
            if bytes.len() > 4096 {
                return Err(io::Error::other("runtime report oversized"));
            }
            fs::write("/dev/termination-log", bytes)
        }
        _ => Err(io::Error::other("unsupported runtime operation")),
    }
}

async fn trusted_image_operation(operation: &str) -> Option<io::Result<()>> {
    let workspace = Path::new("/workspace");
    Some(match operation {
        "builder-probe" => {
            ayni_orchestrator_worker::profiles::ayni::builder_test::probe(workspace).await
        }
        "builder-test-build" => {
            ayni_orchestrator_worker::profiles::ayni::builder_test::build_fixture(workspace).await
        }
        "builder-package" => {
            ayni_orchestrator_worker::operations::builder_package::build(workspace).await
        }
        "publish" => match read_bounded(Path::new("/request/execution.json"), 65536)
            .and_then(|bytes| serde_json::from_slice(&bytes).map_err(io::Error::other))
        {
            Ok(request) => {
                ayni_orchestrator_worker::profiles::ayni::publish(&request, workspace).await
            }
            Err(error) => Err(error),
        },
        _ => return None,
    })
}
