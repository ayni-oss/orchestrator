//! Every runtime command uses shared process ownership and byte capture.
use crate::{Failure, client::WorkerClient};
use ayni_orchestrator_events::execution_diagnostics::OutputStream;
use std::{collections::BTreeMap, path::Path, process::Stdio, time::Duration};
use tokio::process::Command;
const OUTPUT_BYTES: usize = 1024 * 1024;
pub struct Captured {
    pub success: bool,
    pub stdout: Vec<u8>,
    registry_denied: bool,
}
impl Captured {
    #[must_use]
    pub const fn registry_failure(&self) -> Failure {
        if self.registry_denied {
            Failure::RegistryAccessDenied
        } else {
            Failure::RegistryUnavailable
        }
    }
}
fn command(
    program: &str,
    args: &[String],
    directory: &Path,
    environment: &BTreeMap<String, String>,
) -> Command {
    let mut c = Command::new(program);
    c.args(args)
        .current_dir(directory)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("LANG", "C.UTF-8")
        .env("TMPDIR", "/tmp")
        .envs(environment)
        .stdin(Stdio::null());
    c
}
/// # Errors
/// Return safe startup, execution, deadline or artifact-bound failures.
pub async fn run(
    program: &str,
    args: &[String],
    directory: &Path,
    environment: &BTreeMap<String, String>,
    seconds: u64,
) -> Result<Captured, Failure> {
    if seconds == 0 || !Path::new(program).is_absolute() {
        return Err(Failure::InvalidInput);
    }
    let output = ayni_orchestrator_execution::process::run(
        command(program, args, directory, environment),
        Duration::from_secs(seconds),
        OUTPUT_BYTES,
    )
    .await
    .map_err(|_| Failure::ExecutionFailed)?;
    if output.outcome != "exited" {
        return Err(Failure::ExecutionDeadline);
    }
    if output.exceeded {
        return Err(Failure::ExecutionOutputExceeded);
    }
    let diagnostic = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    let registry_denied = [
        "unauthorized",
        "authentication required",
        "access denied",
        "requested access to the resource is denied",
    ]
    .iter()
    .any(|p| diagnostic.contains(p));
    Ok(Captured {
        success: output.status.success(),
        stdout: output.stdout,
        registry_denied,
    })
}
pub(crate) async fn checked_build(
    client: &WorkerClient,
    args: &[String],
    directory: &Path,
    environment: &BTreeMap<String, String>,
    seconds: u64,
) -> Result<(), Failure> {
    // Safe diagnostics remain an independent, bounded observer of the full stream.
    // A slow diagnostic endpoint never holds a child pipe or the raw spool hostage.
    let (out_tx, out_rx) = tokio::sync::mpsc::channel(64);
    let (err_tx, err_rx) = tokio::sync::mpsc::channel(64);
    let lost = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observer_loss = lost.clone();
    let observer: ayni_orchestrator_execution::process::Observer =
        std::sync::Arc::new(move |stream, bytes| {
            let sender = if stream == "stdout" { &out_tx } else { &err_tx };
            if sender.try_send(bytes.to_vec()).is_err() {
                observer_loss.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        });
    let process = ayni_orchestrator_execution::process::run_observed(
        command("/usr/local/bin/ayni", args, directory, environment),
        Duration::from_secs(seconds),
        OUTPUT_BYTES,
        None,
        Some(observer),
    );
    let (output, (), ()) = tokio::join!(
        process,
        build_output::observe(client, OutputStream::Stdout, out_rx, lost.clone()),
        build_output::observe(client, OutputStream::Stderr, err_rx, lost)
    );
    // This build pod has an exclusive Podman store. Detached nested workloads
    // need explicit engine cleanup in addition to POSIX process-group signals.
    let cleanup = ayni_orchestrator_execution::process::run_cleanup(
        command(
            "/usr/bin/podman",
            &[
                "rm".into(),
                "--all".into(),
                "--force".into(),
                "--time".into(),
                "2".into(),
            ],
            directory,
            environment,
        ),
        Duration::from_secs(10),
    )
    .await;
    let output = output.map_err(|_| Failure::ExecutionFailed)?;
    if !cleanup.is_ok_and(|result| result.status.success() && result.outcome == "exited") {
        return Err(Failure::CleanupPending);
    }
    if output.outcome != "exited" {
        Err(Failure::ExecutionDeadline)
    } else if output.status.success() {
        Ok(())
    } else {
        Err(Failure::ExecutionFailed)
    }
}
mod build_output;
/// # Errors
/// Require success without returning untrusted tool diagnostics.
pub async fn checked(
    program: &str,
    args: &[String],
    directory: &Path,
    environment: &BTreeMap<String, String>,
    seconds: u64,
) -> Result<Vec<u8>, Failure> {
    let output = run(program, args, directory, environment, seconds).await?;
    if !output.success {
        return Err(Failure::ExecutionFailed);
    }
    Ok(output.stdout)
}
#[cfg(test)]
mod tests {
    use super::run;
    use std::{collections::BTreeMap, path::Path};
    #[tokio::test]
    async fn trusted_children_use_the_bounded_writable_temporary_mount()
    -> Result<(), Box<dyn std::error::Error>> {
        let output = run(
            "/bin/sh",
            &["-c".into(), "printf '%s' \"$TMPDIR\"".into()],
            Path::new("/tmp"),
            &BTreeMap::new(),
            5,
        )
        .await?;
        assert!(output.success);
        assert_eq!(output.stdout, b"/tmp");
        Ok(())
    }
}
