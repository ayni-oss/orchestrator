//! Repository commands have no access to worker credentials or retained output.
use std::{io, path::Path, process::Command};

/// Construct a child launcher that restricts filesystem and process authority.
/// # Errors
/// Fails when the trusted launcher cannot be resolved.
pub fn command(program: &str, workspace: &Path) -> io::Result<Command> {
    let launcher = std::env::current_exe()?;
    let mut child = super::runtime::command(
        launcher
            .to_str()
            .ok_or_else(|| io::Error::other("launcher path is not UTF-8"))?,
        workspace,
    );
    child.args(["sandbox", program]);
    Ok(child)
}

/// Apply the mandatory sandbox and replace this helper process with its child.
/// # Errors
/// Rejects unsupported kernels or partially enforced rules; there is no fallback.
pub fn execute(program: &str, arguments: &[String]) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        use landlock::{
            ABI, Access, AccessFs, CompatLevel, Compatible, PathBeneath, PathFd, Ruleset,
            RulesetAttr, RulesetCreatedAttr, RulesetStatus, Scope,
        };
        use std::os::fd::AsFd as _;
        use std::os::unix::process::CommandExt;
        let interpreter = native_interpreter()?;
        let abi = ABI::V6;
        let mut rules = Ruleset::default()
            .set_compatibility(CompatLevel::HardRequirement)
            .handle_access(AccessFs::from_all(abi))
            .map_err(io::Error::other)?
            .scope(Scope::Signal | Scope::AbstractUnixSocket)
            .map_err(io::Error::other)?
            .create()
            .map_err(io::Error::other)?;
        for path in [
            "/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc", "/opt", "/home", "/proc",
        ] {
            if Path::new(path).exists() {
                rules = rules
                    .add_rule(PathBeneath::new(
                        PathFd::new(path).map_err(io::Error::other)?,
                        AccessFs::from_read(abi),
                    ))
                    .map_err(io::Error::other)?;
            }
        }
        for path in ["/workspace", "/scratch", "/tmp", "/dev"] {
            if Path::new(path).exists() {
                rules = rules
                    .add_rule(PathBeneath::new(
                        PathFd::new(path).map_err(io::Error::other)?,
                        AccessFs::from_all(abi),
                    ))
                    .map_err(io::Error::other)?;
            }
        }
        rules = rules
            .add_rule(PathBeneath::new(
                PathFd::new(std::env::current_exe()?).map_err(io::Error::other)?,
                AccessFs::Execute | AccessFs::ReadFile,
            ))
            .map_err(io::Error::other)?;
        if let Some(interpreter) = interpreter.as_ref() {
            rules = rules
                .add_rule(PathBeneath::new(
                    interpreter.as_fd(),
                    AccessFs::Execute | AccessFs::ReadFile,
                ))
                .map_err(io::Error::other)?;
        }
        let status = rules.restrict_self().map_err(io::Error::other)?;
        if status.ruleset != RulesetStatus::FullyEnforced || !status.no_new_privs {
            return Err(io::Error::other("repository sandbox is not fully enforced"));
        }
        Err(Command::new(program).args(arguments).exec())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (program, arguments);
        Err(io::Error::other(
            "repository sandbox requires the admitted Linux kernel",
        ))
    }
}

#[cfg(target_os = "linux")]
fn native_interpreter() -> io::Result<Option<landlock::PathFd>> {
    use std::os::fd::{AsFd as _, AsRawFd as _};
    use std::process::Stdio;
    let machine = native_machine();
    if machine == 0 {
        return Err(io::Error::other("unsupported native sandbox architecture"));
    }
    if elf_machine(Path::new("/bin/true"))? == machine {
        return Ok(None);
    }
    // Binfmt may hold an interpreter outside this mount namespace (Rosetta).
    // Resolve only the trusted system process's executable inode, grant that
    // single file read/execute, and stop the probe before restricting ourselves.
    let mut probe = Command::new("/bin/sleep")
        .arg("30")
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let result = (|| {
        let path = std::path::PathBuf::from(format!("/proc/{}/exe", probe.id()));
        let interpreter = landlock::PathFd::new(path).map_err(io::Error::other)?;
        let descriptor = format!("/proc/self/fd/{}", interpreter.as_fd().as_raw_fd());
        if elf_machine(Path::new(&descriptor))? != machine {
            return Err(io::Error::other("system interpreter is not native"));
        }
        Ok(Some(interpreter))
    })();
    let _ = probe.kill();
    probe.wait()?;
    result
}

#[cfg(target_os = "linux")]
fn native_machine() -> u16 {
    match std::env::consts::ARCH {
        "aarch64" => 183,
        "x86_64" => 62,
        _ => 0,
    }
}

#[cfg(target_os = "linux")]
fn elf_machine(path: &Path) -> io::Result<u16> {
    use std::{fs::File, io::Read as _};
    let mut header = [0; 20];
    File::open(path)?.read_exact(&mut header)?;
    if &header[..4] != b"\x7fELF" || header[5] != 1 {
        return Err(io::Error::other("system executable has unsupported format"));
    }
    Ok(u16::from_le_bytes([header[18], header[19]]))
}
