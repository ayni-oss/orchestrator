use ayni_orchestrator_profile_ayni::runtime_execution::ExecutionRequest;
use ayni_orchestrator_runtime::{activity::Stage, runtime_execution::read_bounded};
use ayni_orchestrator_worker::operations::runtime::checkout;
use std::{env, fs, io, path::Path, process::ExitCode};
use tracing::Instrument;

fn main() -> ExitCode {
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
    let telemetry = ayni_orchestrator_observability::init("runtime-worker");
    let result = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_or_else(
            |_| ExitCode::FAILURE,
            |runtime| runtime.block_on(worker_main()),
        );
    telemetry.shutdown();
    result
}

async fn project_worker_main() -> ExitCode {
    let result = async {
        let bytes = read_bounded(Path::new("/request/project-agent.json"), 512 * 1024)?;
        let request: ayni_orchestrator_runtime::project_agent::ProjectAgentRequest =
            serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        if env::args().nth(1).as_deref() == Some("project-checkout") {
            ayni_orchestrator_worker::project_agent::checkout(&request).await?;
        } else {
            let result = ayni_orchestrator_worker::project_agent::run(request).await?;
            println!(
                "AYNI_PROJECT_RESULT:{}",
                serde_json::to_string(&result).map_err(io::Error::other)?
            );
        }
        Ok::<(), io::Error>(())
    }
    .await;
    if result.is_ok() {
        ExitCode::SUCCESS
    } else {
        eprintln!("project_agent_failed");
        ExitCode::FAILURE
    }
}

async fn worker_main() -> ExitCode {
    if matches!(
        env::args().nth(1).as_deref(),
        Some("project-agent" | "project-checkout")
    ) {
        return project_worker_main().await;
    }

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
                    Some("publish" | "build" | "verify")
                ) {
                    let message = error.to_string();
                    let fallback = match env::args().nth(1).as_deref() {
                        Some("publish") => "runtime_publication_failed",
                        Some("build") => "runtime_build_failed",
                        _ => "runtime_verification_failed",
                    };
                    let code = match message.as_str() {
                        "runtime_candidate_missing"
                        | "runtime_lock_mismatch"
                        | "runtime_platform_mismatch"
                        | "runtime_version_unsupported"
                        | "runtime_certificate_invalid"
                        | "engine_tls_or_startup_failed"
                        | "runtime_request_invalid"
                        | "runtime_evidence_unavailable"
                        | "runtime_report_serialization_failed"
                        | "runtime_report_oversized"
                        | "runtime_reporting_failed" => message.as_str(),
                        _ => fallback,
                    };
                    let _ = fs::write("/dev/termination-log", code);
                }
                if ayni_orchestrator_worker::operation::enabled()
                    && !Path::new("/output/runtime-operation.json").exists()
                {
                    let reason = match error.to_string().as_str() {
                        "runtime_candidate_missing" => "runtime_candidate_missing",
                        "runtime_lock_mismatch" => "runtime_lock_mismatch",
                        "runtime_platform_mismatch" => "runtime_platform_mismatch",
                        "runtime_version_unsupported" => "runtime_version_unsupported",
                        "engine_tls_or_startup_failed" => "engine_tls_or_startup_failed",
                        _ => match env::args().nth(1).as_deref() {
                            Some("trust-lock") => "certificate_trust_lock_failed",
                            Some("build") => "runtime_build_failed",
                            Some("publish") => "runtime_publication_failed",
                            _ => "runtime_execution_failed",
                        },
                    };
                    let _ = ayni_orchestrator_worker::operation::complete(
                        ayni_orchestrator_runtime::operation::OperationOutcome::Failed {
                            reason: reason.into(),
                        },
                    )
                    .await;
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
    if ayni_orchestrator_worker::operation::enabled()
        && (operation == "report-operation" || Path::new("/output/runtime-operation.json").exists())
    {
        return ayni_orchestrator_worker::operation::resume().await;
    }
    if operation == "registry-discover" {
        return ayni_orchestrator_worker::operations::image_discovery::run().await;
    }
    if let Some(result) = utility_operation(&operation, activity) {
        return result;
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
    if operation == "report-verification" {
        let admission = serde_json::from_slice(&read_bounded(
            Path::new("/request/reporting.json"),
            ayni_orchestrator_events::MAX_EVENT_BYTES,
        )?)
        .map_err(io::Error::other)?;
        let pod_uid =
            env::var("POD_UID").map_err(|_| io::Error::other("runtime_request_invalid"))?;
        return ayni_orchestrator_worker::verification::reporting::deliver(
            Path::new("/output"),
            &admission,
            &pod_uid,
            Path::new("/reporting/token"),
        )
        .await;
    }
    if operation == "verify" {
        activity.record(Stage::RuntimeVerification);
        return ayni_orchestrator_worker::verification::run().await;
    }
    let request: ExecutionRequest =
        serde_json::from_slice(&read_bounded(Path::new("/request/execution.json"), 65_536)?)
            .map_err(io::Error::other)?;
    match operation.as_str() {
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
        _ => Err(io::Error::other("unsupported runtime operation")),
    }
}

async fn trusted_image_operation(operation: &str) -> Option<io::Result<()>> {
    let workspace = Path::new("/workspace");
    Some(match operation {
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

fn utility_operation(
    operation: &str,
    activity: &ayni_orchestrator_worker::coding::activity::ActivityLog,
) -> Option<io::Result<()>> {
    if !matches!(operation, "install" | "prepare-output") {
        return None;
    }
    Some((|| {
        if operation == "install" {
            activity.record(Stage::Install);
            fs::copy(env::current_exe()?, "/launcher/runtime-worker")?;
            fs::copy("/usr/local/bin/mise", "/launcher/mise")?;
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
        Err(io::Error::other("unsupported utility operation"))
    })())
}
