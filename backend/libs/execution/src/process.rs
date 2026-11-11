//! Process groups, concurrent byte capture and bounded shutdown.
use crate::{Capture, capture};
use std::{
    io,
    process::{ExitStatus, Stdio},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt as _},
    process::Command,
};

pub type Observer = std::sync::Arc<dyn Fn(&str, &[u8]) + Send + Sync>;

pub struct Output {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exceeded: bool,
    pub outcome: &'static str,
}

async fn drain(
    mut pipe: impl AsyncRead + Unpin,
    capture: Option<Capture>,
    action: String,
    stream: &'static str,
    limit: usize,
    observer: Option<Observer>,
) -> io::Result<(Vec<u8>, bool)> {
    let mut retained = Vec::new();
    let mut exceeded = false;
    let mut buffer = [0; 8192];
    loop {
        let count = pipe.read(&mut buffer).await?;
        if count == 0 {
            return Ok((retained, exceeded));
        }
        if let Some(c) = &capture {
            let _ = c.record(&action, "output", stream, &buffer[..count], None, None);
        }
        if let Some(observer) = &observer {
            observer(stream, &buffer[..count]);
        }
        let keep = count.min(limit.saturating_sub(retained.len()));
        retained.extend_from_slice(&buffer[..keep]);
        exceeded |= keep < count;
    }
}

fn signal_group(id: Option<u32>, signal: rustix::process::Signal) {
    if let Some(pid) = id
        .and_then(|id| i32::try_from(id).ok())
        .and_then(rustix::process::Pid::from_raw)
    {
        let _ = rustix::process::kill_process_group(pid, signal);
    }
}

struct GroupGuard(Option<u32>);
impl Drop for GroupGuard {
    fn drop(&mut self) {
        signal_group(self.0, rustix::process::Signal::KILL);
    }
}

/// Kill the owned process group, never just the immediate command.
pub fn kill_group(id: u32) {
    signal_group(Some(id), rustix::process::Signal::KILL);
}

pub async fn termination() {
    #[cfg(unix)]
    {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! { _=signal.recv()=>{}, _=tokio::signal::ctrl_c()=>{} }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

/// Run one command while the worker remains alive. Diagnostic loss is independent
/// of execution; callers separately validate bounded machine-readable artifacts.
/// # Errors
/// Returns spawn, wait or pipe failures after terminating the owned process group.
#[allow(
    clippy::too_many_lines,
    reason = "keep process ownership and all cleanup exits in one audited scope"
)]
pub async fn run_observed(
    command: Command,
    deadline: Duration,
    retain_bytes: usize,
    input: Option<Vec<u8>>,
    observer: Option<Observer>,
) -> io::Result<Output> {
    supervise(command, deadline, retain_bytes, input, observer, false).await
}

/// Run trusted teardown even after cancellation; the cleanup deadline still applies.
/// # Errors
/// Return spawn, wait or drain failures after bounded cleanup.
pub async fn run_cleanup(command: Command, deadline: Duration) -> io::Result<Output> {
    supervise(command, deadline, 8192, None, None, true).await
}

#[allow(
    clippy::too_many_lines,
    reason = "process ownership and cleanup share one scope"
)]
async fn supervise(
    mut command: Command,
    deadline: Duration,
    retain_bytes: usize,
    input: Option<Vec<u8>>,
    observer: Option<Observer>,
    cleanup: bool,
) -> io::Result<Output> {
    #[cfg(target_os = "linux")]
    rustix::process::set_child_subreaper(Some(rustix::process::getpid()))
        .map_err(io::Error::other)?;
    if !cleanup && crate::is_cancelled() {
        return Err(io::Error::other("execution cancelled"));
    }
    let action = uuid::Uuid::new_v4().to_string();
    let capture = capture();
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            if let Some(c) = &capture {
                let _ = c.record(&action, "spawn_failed", "", &[], None, None);
            }
            return Err(error);
        }
    };
    let began = std::time::Instant::now();
    let id = child.id();
    let mut group = GroupGuard(id);
    let writer = child.stdin.take().map(|mut stdin| {
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt as _;
            if let Some(input) = input {
                stdin.write_all(&input).await?;
            }
            Ok::<(), io::Error>(())
        })
    });
    if let Some(c) = &capture {
        let _ = c
            .spool
            .append(ayni_orchestrator_telemetry::v1::TelemetryRecord {
                source: Some(c.source.clone()),
                action_id: action.clone(),
                parent_action_id: format!(
                    "{}:{}:{}",
                    c.source.execution_id, c.source.stage, c.source.generation
                ),
                event: "started".into(),
                observed_at_unix_ms: crate::now_ms(),
                deadline_unix_ms: crate::now_ms()
                    .saturating_add(i64::try_from(deadline.as_millis()).unwrap_or(i64::MAX)),
                command_digest: c.command_digest.clone(),
                ..Default::default()
            });
    }
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("stdout pipe unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("stderr pipe unavailable"))?;
    let mut out = tokio::spawn(drain(
        stdout,
        capture.clone(),
        action.clone(),
        "stdout",
        retain_bytes,
        observer.clone(),
    ));
    let mut err = tokio::spawn(drain(
        stderr,
        capture.clone(),
        action.clone(),
        "stderr",
        retain_bytes,
        observer.clone(),
    ));
    let (status, outcome) = tokio::select! {
        status=child.wait()=> (status,"exited"),
        ()=tokio::time::sleep(deadline)=> (stop(&mut child,id).await,"timed_out"),
        ()=termination()=> (stop(&mut child,id).await,"cancelled"),
        ()=crate::cancelled(), if !cleanup=> (stop(&mut child,id).await,"cancelled"),
    };
    // A command may leave descendants holding pipes even after it exits.
    signal_group(id, rustix::process::Signal::TERM);
    let drained = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(&mut out, &mut err)
    })
    .await;
    signal_group(id, rustix::process::Signal::KILL);
    group.0 = None;
    if let Some(writer) = writer {
        writer.abort();
    }
    let status = status?;
    #[cfg(target_os = "linux")]
    if let Some(group) = id
        .and_then(|n| i32::try_from(n).ok())
        .and_then(rustix::process::Pid::from_raw)
    {
        // Only reap this action's adopted descendants; other commands retain their waits.
        for _ in 0..20 {
            match rustix::process::waitpgid(group, rustix::process::WaitOptions::NOHANG) {
                Ok(Some(_)) => continue,
                Ok(None) => tokio::time::sleep(Duration::from_millis(10)).await,
                Err(_) => break,
            }
        }
    }
    #[cfg(unix)]
    let signal = {
        use std::os::unix::process::ExitStatusExt as _;
        status.signal()
    };
    #[cfg(not(unix))]
    let signal = None;
    let event = if outcome != "exited" {
        outcome
    } else if signal.is_some() {
        "signalled"
    } else if status.success() {
        "succeeded"
    } else {
        "failed"
    };
    if let Some(c) = &capture {
        let _ = c
            .spool
            .append(ayni_orchestrator_telemetry::v1::TelemetryRecord {
                source: Some(c.source.clone()),
                action_id: action.clone(),
                parent_action_id: format!(
                    "{}:{}:{}",
                    c.source.execution_id, c.source.stage, c.source.generation
                ),
                event: event.into(),
                observed_at_unix_ms: crate::now_ms(),
                duration_ms: u64::try_from(began.elapsed().as_millis()).unwrap_or(u64::MAX),
                exit_code: status.code(),
                signal,
                command_digest: c.command_digest.clone(),
                ..Default::default()
            });
    }
    let (stdout, stderr) = if let Ok((out, err)) = drained {
        (
            out.map_err(io::Error::other)??,
            err.map_err(io::Error::other)??,
        )
    } else {
        out.abort();
        err.abort();
        if let Some(c) = &capture {
            let _ = c.record(&action, "drain_incomplete", "", &[], status.code(), signal);
        }
        return Err(io::Error::other("output drain incomplete"));
    };
    if let Some(c) = &capture {
        let _ = c.record(&action, "output_drained", "", &[], status.code(), signal);
    }
    Ok(Output {
        status,
        stdout: stdout.0,
        stderr: stderr.0,
        exceeded: stdout.1 || stderr.1,
        outcome,
    })
}

async fn stop(child: &mut tokio::process::Child, id: Option<u32>) -> io::Result<ExitStatus> {
    signal_group(id, rustix::process::Signal::TERM);
    if let Ok(status) = tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
        return status;
    }
    signal_group(id, rustix::process::Signal::KILL);
    child.wait().await
}

/// # Errors
/// Return startup, wait or drain failures after process group cleanup.
pub async fn run_with_input(
    command: Command,
    deadline: Duration,
    retain_bytes: usize,
    input: Option<Vec<u8>>,
) -> io::Result<Output> {
    run_observed(command, deadline, retain_bytes, input, None).await
}

/// # Errors
/// Return command startup, wait or drain failures after cleanup.
pub async fn run(command: Command, deadline: Duration, retain_bytes: usize) -> io::Result<Output> {
    run_with_input(command, deadline, retain_bytes, None).await
}

/// Synchronous trusted launchers share the same process and capture contract.
/// # Errors
/// Return execution errors or an unavailable supervision thread/runtime.
pub fn run_blocking(
    command: std::process::Command,
    deadline: Duration,
    retain_bytes: usize,
) -> io::Result<Output> {
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(run(Command::from(command), deadline, retain_bytes))
    })
    .join()
    .map_err(|_| io::Error::other("supervision thread unavailable"))?
}

/// # Errors
/// Return owned command failures after cleanup.
pub fn run_blocking_with_input(
    command: std::process::Command,
    deadline: Duration,
    retain_bytes: usize,
    input: Option<Vec<u8>>,
) -> io::Result<Output> {
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(run_with_input(
                Command::from(command),
                deadline,
                retain_bytes,
                input,
            ))
    })
    .join()
    .map_err(|_| io::Error::other("supervision thread unavailable"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn both_streams_and_repeated_commands_survive_nonzero_exit() -> io::Result<()> {
        for _ in 0..2 {
            let mut command = Command::new("/bin/sh");
            command.args(["-c", "printf 'same\\nsame\\n'; printf 'error' >&2; exit 7"]);
            let output = run(command, Duration::from_secs(5), 1024).await?;
            assert_eq!(output.status.code(), Some(7));
            assert_eq!(output.stdout, b"same\nsame\n");
            assert_eq!(output.stderr, b"error");
        }
        Ok(())
    }
    #[tokio::test]
    async fn timeout_terminates_descendants_holding_pipes() -> io::Result<()> {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "sleep 60 & wait"]);
        let output = run(command, Duration::from_millis(50), 1024).await?;
        assert_eq!(output.outcome, "timed_out");
        Ok(())
    }
    #[tokio::test]
    async fn cancelled_execution_still_allows_bounded_trusted_cleanup() -> io::Result<()> {
        if std::env::var_os("AYNI_CANCELLATION_TEST_CHILD").is_none() {
            let mut command = Command::new(std::env::current_exe()?);
            command
                .args([
                    "--exact",
                    "process::tests::cancelled_execution_still_allows_bounded_trusted_cleanup",
                ])
                .env("AYNI_CANCELLATION_TEST_CHILD", "1");
            let output = run(command, Duration::from_secs(10), 8192).await?;
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
            return Ok(());
        }
        crate::request_cancellation();
        assert!(
            run(Command::new("/usr/bin/true"), Duration::from_secs(1), 1024)
                .await
                .is_err()
        );
        let output = run_cleanup(Command::new("/usr/bin/true"), Duration::from_secs(1)).await?;
        assert!(output.status.success());
        let mut slow = Command::new("/bin/sleep");
        slow.arg("60");
        assert_eq!(
            run_cleanup(slow, Duration::from_millis(20)).await?.outcome,
            "timed_out"
        );
        Ok(())
    }
    #[tokio::test]
    async fn capture_limit_does_not_block_pipe_drain() -> io::Result<()> {
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            "head -c 131072 /dev/zero; head -c 131072 /dev/zero >&2",
        ]);
        let output = run(command, Duration::from_secs(5), 1024).await?;
        assert!(output.status.success() && output.exceeded);
        assert_eq!(output.stdout.len(), 1024);
        assert_eq!(output.stderr.len(), 1024);
        Ok(())
    }
}
