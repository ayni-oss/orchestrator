//! A short-lived stage worker. Kubernetes retires the entire cgroup before the
//! authority can advance; no descendant survives into a more privileged stage.
use ayni_orchestrator_events::runtime_preparation::{Binding, StageReport};
use ayni_orchestrator_runtime_preparation::{Failure, client::WorkerClient, pipeline};
use std::{env, fs, process::ExitCode};
mod compatibility;

fn configured(name: &str) -> Result<String, Failure> {
    env::var(name).map_err(|_| Failure::InvalidInput)
}

fn main() -> ExitCode {
    let arguments: Vec<_> = env::args().skip(1).collect();
    match arguments
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["--install-probes"] => {
            if compatibility::install_probes().is_ok() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            }
        }
        ["--replay-telemetry"] => tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_or(ExitCode::FAILURE, |runtime| {
                if runtime
                    .block_on(ayni_orchestrator_runtime_preparation::telemetry::replay())
                    .is_ok()
                {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                }
            }),
        ["--compat-launch"] => compatibility::launch(),
        ["--compat-prepare"] => compatibility::prepare(),
        ["--compat-child", stage] => compatibility::child(stage),
        [] => tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_or_else(
                |_| ExitCode::from(2),
                |runtime| {
                    if runtime.block_on(run()).is_ok() {
                        ExitCode::SUCCESS
                    } else {
                        eprintln!("runtime stage requires recovery");
                        ExitCode::from(2)
                    }
                },
            ),
        _ => ExitCode::from(2),
    }
}
async fn run() -> Result<(), Failure> {
    let binding = Binding {
        operation_id: configured("ORCHESTRATOR_RUNTIME_OPERATION")?,
        stage: configured("ORCHESTRATOR_RUNTIME_STAGE")?,
        generation: configured("ORCHESTRATOR_RUNTIME_GENERATION")?
            .parse()
            .map_err(|_| Failure::InvalidInput)?,
        pod_uid: configured("ORCHESTRATOR_RUNTIME_POD_UID")?,
        command_digest: configured("ORCHESTRATOR_RUNTIME_COMMAND_DIGEST")?,
    };
    let capability = fs::read_to_string(configured("ORCHESTRATOR_RUNTIME_CAPABILITY_PATH")?)
        .map_err(|_| Failure::InvalidInput)?;
    let client = WorkerClient::new(
        &configured("ORCHESTRATOR_RUNTIME_AUTHORITY_URL")?,
        capability.trim().into(),
        binding.clone(),
    )?;
    // The pod can start while the supervisor is recording the observed UID. It
    // must wait for that authority fence before acquiring credentials or inputs.
    let mut claim = None;
    for _ in 0..60 {
        match client.input().await {
            Ok(input) => {
                claim = Some(input);
                break;
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(500)).await,
        }
    }
    let claim = claim.ok_or(Failure::ExecutionFailed)?;
    let telemetry = ayni_orchestrator_runtime_preparation::telemetry::Delivery::start(
        &binding,
        capability.trim().into(),
    )
    .unwrap_or_else(|_| {
        eprintln!("runtime telemetry unavailable; capture incomplete");
        None
    });
    let report = match pipeline::execute(&client, &claim).await {
        Ok(result) => StageReport {
            binding: client.binding.clone(),
            outcome: result.outcome.into(),
            safe_reason: String::new(),
            evidence: result.evidence,
        },
        Err(reason) => pipeline::failure_report(&client, reason),
    };
    if let Some(telemetry) = telemetry {
        telemetry.finish().await;
    }
    if client.binding.stage == "certify" {
        // Only the trusted supervisor can recheck the mounted Secret UID after
        // this workload terminates, then accept the exact certification report.
        let json = serde_json::to_string(&report).map_err(|_| Failure::ArtifactInvalid)?;
        if json.len() > 32 * 1024 {
            return Err(Failure::ExecutionOutputExceeded);
        }
        println!("{json}");
        return Ok(());
    }
    let mut accepted = false;
    for _ in 0..3 {
        if client.report(report.clone()).await.is_ok() {
            accepted = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    if accepted {
        Ok(())
    } else {
        Err(Failure::ExecutionFailed)
    }
}
