//! Own subprocess groups so cancellation cannot leave shell descendants running.
use std::{
    io,
    ops::{Deref, DerefMut},
    process::ExitStatus,
    time::Duration,
};
use tokio::process::{Child, Command};

pub struct Process {
    child: Child,
    #[cfg(unix)]
    group: Option<rustix::process::Pid>,
}

pub fn spawn(command: &mut Command) -> io::Result<Process> {
    command.kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let child = command.spawn()?;
    #[cfg(unix)]
    let group = child
        .id()
        .and_then(|id| i32::try_from(id).ok())
        .and_then(rustix::process::Pid::from_raw);
    Ok(Process {
        child,
        #[cfg(unix)]
        group,
    })
}

impl Process {
    fn stop_group(&mut self) {
        #[cfg(unix)]
        if let Some(group) = self.group.take() {
            // A command may finish while a descendant still owns its pipes.
            // Cancel the owned group on success, failure and dropped futures.
            let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
        }
    }
    pub async fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = self.child.wait().await?;
        self.stop_group();
        Ok(status)
    }
    async fn terminate(&mut self) -> io::Result<()> {
        self.stop_group();
        self.child.kill().await
    }
}
impl Deref for Process {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.child
    }
}
impl DerefMut for Process {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.child
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        self.stop_group();
    }
}

pub async fn status(command: &mut Command, deadline: Duration) -> io::Result<ExitStatus> {
    let mut process = spawn(command)?;
    if let Ok(result) = tokio::time::timeout(deadline, process.wait()).await {
        result
    } else {
        process.terminate().await?;
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "worker command deadline exceeded",
        ))
    }
}

/// Blocking Git/checkpoint work remains bounded even after an async caller disconnects.
pub struct SynchronousProcess {
    child: std::process::Child,
    #[cfg(unix)]
    group: Option<rustix::process::Pid>,
}

pub fn spawn_sync(command: &mut std::process::Command) -> io::Result<SynchronousProcess> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    let child = command.spawn()?;
    #[cfg(unix)]
    let group = i32::try_from(child.id())
        .ok()
        .and_then(rustix::process::Pid::from_raw);
    Ok(SynchronousProcess {
        child,
        #[cfg(unix)]
        group,
    })
}
impl SynchronousProcess {
    fn stop_group(&mut self) {
        #[cfg(unix)]
        if let Some(group) = self.group.take() {
            let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
        }
    }
    pub fn wait(&mut self, deadline: Duration) -> io::Result<ExitStatus> {
        let started = std::time::Instant::now();
        loop {
            if let Some(status) = self.child.try_wait()? {
                self.stop_group();
                return Ok(status);
            }
            if started.elapsed() >= deadline {
                self.stop_group();
                let _ = self.child.kill();
                let _ = self.child.wait();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "worker command deadline exceeded",
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Deref for SynchronousProcess {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.child
    }
}
impl DerefMut for SynchronousProcess {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.child
    }
}
impl Drop for SynchronousProcess {
    fn drop(&mut self) {
        self.stop_group();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::path::Path;
    fn descendants(marker: &Path, ready: &Path) -> Command {
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                "(sleep 0.3; printf leaked > \"$1\") & printf '%s' \"$!\" > \"$2\"; wait",
                "test",
            ])
            .arg(marker)
            .arg(ready);
        command
    }
    async fn wait_ready(ready: &Path) -> io::Result<()> {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !ready.exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .map_err(io::Error::other)
    }
    #[tokio::test]
    async fn cancelling_a_running_future_stops_descendants() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let marker = directory.path().join("marker");
        let ready = directory.path().join("ready");
        let mut command = descendants(&marker, &ready);
        let task = tokio::spawn(async move { status(&mut command, Duration::from_secs(5)).await });
        wait_ready(&ready).await?;
        task.abort();
        assert!(task.await.is_err_and(|error| error.is_cancelled()));
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(!marker.exists());
        Ok(())
    }
    #[tokio::test]
    async fn deadline_stops_descendants_and_reports_timeout() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let marker = directory.path().join("marker");
        let ready = directory.path().join("ready");
        let error = status(
            &mut descendants(&marker, &ready),
            Duration::from_millis(100),
        )
        .await
        .err()
        .ok_or_else(|| io::Error::other("expected timeout"))?;
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(ready.exists());
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(!marker.exists());
        Ok(())
    }
    #[tokio::test]
    async fn successful_leader_exit_stops_background_descendants() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let marker = directory.path().join("marker");
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "(sleep 0.3; printf leaked > \"$1\") & exit 0", "test"])
            .arg(&marker);
        assert!(
            status(&mut command, Duration::from_secs(2))
                .await?
                .success()
        );
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(!marker.exists());
        Ok(())
    }

    #[test]
    fn blocking_deadline_stops_background_descendants() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let marker = directory.path().join("marker");
        let ready = directory.path().join("ready");
        let mut command = std::process::Command::new("/bin/sh");
        command
            .args([
                "-c",
                "(sleep 0.3; printf leaked > \"$1\") & printf ready > \"$2\"; wait",
                "test",
            ])
            .arg(&marker)
            .arg(&ready);
        let error = spawn_sync(&mut command)?
            .wait(Duration::from_millis(100))
            .err()
            .ok_or_else(|| io::Error::other("expected timeout"))?;
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(ready.exists());
        std::thread::sleep(Duration::from_millis(400));
        assert!(!marker.exists());
        Ok(())
    }
}
