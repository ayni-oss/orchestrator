use super::{Failure, MAX_ARTIFACT, WORKSPACE};
use std::{
    fs,
    io::{Read as _, Seek as _, SeekFrom},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[cfg(target_os = "linux")]
pub(super) fn verify_parent() -> Result<(), Failure> {
    use rustix::{
        process,
        thread::{self, CapabilitySet},
    };
    if process::getuid().as_raw() != 0
        || process::geteuid().as_raw() != 0
        || process::getgid().as_raw() != 0
        || process::getegid().as_raw() != 0
    {
        return Err(Failure::InvalidInput);
    }
    let caps = thread::capabilities(None).map_err(|_| Failure::InvalidInput)?;
    let allowed = CapabilitySet::SETUID | CapabilitySet::SETGID;
    if caps.effective != allowed
        || caps.permitted != allowed
        || !(caps.inheritable & !allowed).is_empty()
        || !thread::no_new_privs().map_err(|_| Failure::InvalidInput)?
    {
        return Err(Failure::InvalidInput);
    }
    reject_inherited_sockets()
}
#[cfg(target_os = "linux")]
pub(super) fn reject_inherited_sockets() -> Result<(), Failure> {
    for entry in fs::read_dir("/proc/self/fd").map_err(|_| Failure::InvalidInput)? {
        let entry = entry.map_err(|_| Failure::InvalidInput)?;
        if fs::read_link(entry.path())
            .map_err(|_| Failure::InvalidInput)?
            .to_string_lossy()
            .starts_with("socket:")
        {
            return Err(Failure::InvalidInput);
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub(super) fn capture(stage: &str, deadline: Instant) -> Result<Vec<u8>, Failure> {
    use std::os::unix::{fs::OpenOptionsExt as _, process::CommandExt as _};
    if Instant::now() >= deadline {
        return Err(Failure::ExecutionDeadline);
    }
    let private = tempfile::tempdir_in("/tmp").map_err(|_| Failure::ExecutionFailed)?;
    let open = |name| {
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(private.path().join(name))
    };
    let mut stdout = open("stdout").map_err(|_| Failure::ExecutionFailed)?;
    let stderr = open("stderr").map_err(|_| Failure::ExecutionFailed)?;
    let mut command = Command::new("/trusted/runtime-worker");
    command
        .args(["--compat-child", stage])
        .uid(10002)
        .gid(10002)
        .env_clear()
        .current_dir(WORKSPACE)
        .stdin(Stdio::null())
        .stdout(stdout.try_clone().map_err(|_| Failure::ExecutionFailed)?)
        .stderr(stderr.try_clone().map_err(|_| Failure::ExecutionFailed)?);
    child_environment(&mut command);
    let mut child = command.spawn().map_err(|_| Failure::ExecutionFailed)?;
    let status = loop {
        let oversized = stdout
            .metadata()
            .map_err(|_| Failure::ExecutionFailed)?
            .len()
            > MAX_ARTIFACT as u64
            || stderr
                .metadata()
                .map_err(|_| Failure::ExecutionFailed)?
                .len()
                > 8192;
        if oversized || Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(if oversized {
                Failure::ExecutionOutputExceeded
            } else {
                Failure::ExecutionDeadline
            });
        }
        if let Some(status) = child.try_wait().map_err(|_| Failure::ExecutionFailed)? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    if stdout
        .metadata()
        .map_err(|_| Failure::ExecutionFailed)?
        .len()
        > MAX_ARTIFACT as u64
        || stderr
            .metadata()
            .map_err(|_| Failure::ExecutionFailed)?
            .len()
            > 8192
    {
        return Err(Failure::ExecutionOutputExceeded);
    }
    if stage != "quality" && !status.success()
        || stage == "quality" && !matches!(status.code(), Some(0 | 1))
    {
        return Err(Failure::ExecutionFailed);
    }
    stdout
        .seek(SeekFrom::Start(0))
        .map_err(|_| Failure::ExecutionFailed)?;
    let mut bytes = Vec::new();
    stdout
        .take(MAX_ARTIFACT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::ExecutionFailed)?;
    Ok(bytes)
}

fn child_environment(command: &mut Command) {
    for (name, value) in [
        ("PATH", "/usr/local/bin:/usr/bin:/bin"),
        ("MISE_DATA_DIR", "/opt/ayni/mise"),
        ("MISE_AUTO_INSTALL", "0"),
        ("MISE_TRUSTED_CONFIG_PATHS", "/etc/ayni"),
        ("AYNI_RUNTIME_CARGO_HOME", "/home/ayni/.cache/cargo"),
        ("RUSTUP_HOME", "/home/ayni/.rustup"),
        ("HOME", "/tmp/home"),
        ("XDG_CACHE_HOME", "/tmp/cache"),
        ("MISE_CACHE_DIR", "/tmp/mise"),
        ("CARGO_HOME", "/tmp/cargo"),
        ("CARGO_TARGET_DIR", "/tmp/target"),
        ("CARGO_INCREMENTAL", "0"),
        ("npm_config_cache", "/tmp/npm"),
        ("GRADLE_USER_HOME", "/tmp/gradle"),
        ("UV_CACHE_DIR", "/tmp/uv"),
    ] {
        command.env(name, value);
    }
}
