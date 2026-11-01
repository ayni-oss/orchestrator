//! Bounded child execution. Kubernetes owns the outer process/cgroup deadline;
//! this timeout also kills and reaps the immediate child on interrupted stages.
use crate::Failure;
use std::{collections::BTreeMap, path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt as _},
    process::Command,
};

const OUTPUT_BYTES: u64 = 1024 * 1024;

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

/// Execute a fixed trusted tool with literal arguments and an explicit environment.
///
/// Callers must not place credentials in arguments. Ambient Git configuration,
/// proxy variables, signing variables and credential helpers are not inherited.
/// # Errors
/// Return a closed reason on startup, timeout, IO or output-bound failures.
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
    let mut child = Command::new(program)
        .args(args)
        .current_dir(directory)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("LANG", "C.UTF-8")
        .env("TMPDIR", "/tmp")
        .envs(environment)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| Failure::ToolUnavailable)?;
    let stdout = child.stdout.take().ok_or(Failure::ExecutionFailed)?;
    let stderr = child.stderr.take().ok_or(Failure::ExecutionFailed)?;
    let execution = async {
        let (stdout, stderr) = tokio::try_join!(bounded(stdout), bounded(stderr))?;
        let status = child.wait().await.map_err(|_| Failure::ExecutionFailed)?;
        let diagnostic = String::from_utf8_lossy(&stderr).to_ascii_lowercase();
        let registry_denied = [
            "unauthorized",
            "authentication required",
            "access denied",
            "requested access to the resource is denied",
        ]
        .iter()
        .any(|pattern| diagnostic.contains(pattern));
        Ok(Captured {
            success: status.success(),
            stdout,
            registry_denied,
        })
    };
    let result = tokio::time::timeout(Duration::from_secs(seconds), execution).await;
    match result {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(reason)) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            Err(reason)
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            Err(Failure::ExecutionDeadline)
        }
    }
}

async fn bounded(reader: impl AsyncRead + Unpin) -> Result<Vec<u8>, Failure> {
    let mut bytes = Vec::new();
    reader
        .take(OUTPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| Failure::ExecutionFailed)?;
    if bytes.len() as u64 > OUTPUT_BYTES {
        return Err(Failure::ExecutionOutputExceeded);
    }
    Ok(bytes)
}

/// # Errors
/// Require successful execution without returning untrusted tool diagnostics.
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
