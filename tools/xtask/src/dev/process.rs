//! Owned host processes: bounded cancellation includes descendants.
use anyhow::{Context as _, Result, bail};
use rustix::process::{Pid, Signal, kill_process_group};
use std::{
    fs::File,
    os::unix::process::CommandExt as _,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

pub static STOP: AtomicBool = AtomicBool::new(false);

pub struct Owned(pub Child, bool);
impl Owned {
    pub fn spawn(command: &mut Command) -> Result<Self> {
        command.process_group(0);
        Ok(Self(
            command
                .spawn()
                .with_context(|| format!("starting {}", command.get_program().to_string_lossy()))?,
            false,
        ))
    }
    pub fn stop(&mut self) {
        if self.1 {
            return;
        }
        self.1 = true;
        if let Some(pid) = Pid::from_raw(self.0.id().cast_signed()) {
            let _ = kill_process_group(pid, Signal::TERM);
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if self.0.try_wait().ok().flatten().is_some() {
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
            // Descendants may survive their leader; terminate the entire group.
            let _ = kill_process_group(pid, Signal::KILL);
        }
        let _ = self.0.wait();
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn run(command: &mut Command) -> Result<()> {
    let mut child = Owned::spawn(command)?;
    let build = ["cargo", "docker"].contains(&command.get_program().to_string_lossy().as_ref())
        && command.get_args().any(|arg| arg == "build");
    let mut capacity_checked = Instant::now();
    loop {
        if STOP.load(Ordering::Relaxed) {
            bail!("development controller stopping");
        }
        if let Some(status) = child.0.try_wait()? {
            if !status.success() {
                bail!(
                    "{} exited with {status}",
                    command.get_program().to_string_lossy()
                );
            }
            return Ok(());
        }
        if build && capacity_checked.elapsed() >= Duration::from_secs(5) {
            super::images::capacity()?;
            capacity_checked = Instant::now();
        }
        thread::sleep(Duration::from_millis(100));
    }
}

pub fn output(command: &mut Command) -> Result<String> {
    let file = tempfile::tempfile()?;
    command.stdout(Stdio::from(file.try_clone()?));
    run(command)?;
    read_output(file)
}
fn read_output(mut file: File) -> Result<String> {
    use std::io::{Read as _, Seek as _};
    file.rewind()?;
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    Ok(text.trim().into())
}
