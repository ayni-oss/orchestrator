//! Small, statically linked local-chart entrypoint. No credentials enter files or arguments.
use std::{
    env,
    error::Error,
    fs, io,
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn main() -> Result<()> {
    let args: Vec<_> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("nats") => start_nats(&args[1..]),
        Some("provision") if args.len() == 2 => provision(&args[1]),
        Some("postgres-volume") => postgres_volume(),
        _ => Err("expected nats, provision INSTANCE, or postgres-volume".into()),
    }
}
fn quote(value: &str) -> String {
    let mut result = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\\' => result.push_str("\\\\"),
            '"' => result.push_str("\\\""),
            '\0'..='\u{1f}' | '\u{7f}' => result.push_str(&format!("\\x{:02x}", ch as u32)),
            _ => result.push(ch),
        }
    }
    result.push('"');
    result
}
fn start_nats(args: &[String]) -> Result<()> {
    let mut command = Command::new("nats-server");
    for role in ["ADMIN", "APPLICATION", "EXECUTOR", "ACCEPTANCE"] {
        let name = format!("WORKFLOW_{role}_PASSWORD");
        let value = env::var(&name).map_err(|_| format!("{name} missing"))?;
        if value.is_empty() {
            return Err(format!("{name} empty").into());
        }
        command.env(name, quote(&value));
    }
    Err(command.args(args).exec().into())
}
fn nats() -> Result<Command> {
    let mut command = Command::new("nats");
    command
        .env("NATS_URL", "nats://jetstream:4222")
        .env("NATS_USER", "workflow-admin")
        .env(
            "NATS_PASSWORD",
            env::var("WORKFLOW_ADMIN_PASSWORD").map_err(|_| "broker password missing")?,
        )
        .arg("--timeout=2s");
    Ok(command)
}
fn checked(command: &mut Command) -> Result<()> {
    if !command.status()?.success() {
        return Err("infrastructure command failed".into());
    }
    Ok(())
}
fn provision(instance: &str) -> Result<()> {
    if instance.len() != 36 || !instance.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
        return Err("invalid executor instance".into());
    }
    let deadline = Instant::now() + Duration::from_secs(120);
    while !nats()?
        .args(["account", "info"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?
        .success()
    {
        if Instant::now() >= deadline {
            return Err("broker readiness timed out".into());
        }
        thread::sleep(Duration::from_secs(2));
    }
    let streams = nats()?.args(["stream", "ls", "--names"]).output()?;
    if !streams.status.success() {
        return Err("cannot list broker streams".into());
    }
    let names = String::from_utf8(streams.stdout)?;
    for (name, file) in [
        ("ORCHESTRATOR_WORKFLOW_COMMAND_V1", "command"),
        ("ORCHESTRATOR_WORKFLOW_REPORT_V1", "report"),
        ("ORCHESTRATOR_WORKFLOW_DECISION_V1", "decision"),
        ("ORCHESTRATOR_EXECUTION_DIAGNOSTICS_V1", "diagnostics"),
    ] {
        let config = format!("--config=/config/{file}.json");
        if names.lines().any(|line| line.trim() == name) {
            checked(nats()?.args(["stream", "edit", name, &config, "--force"]))?;
        } else {
            checked(nats()?.args(["stream", "add", &config, "--defaults"]))?;
        }
    }
    for (stream, consumer, file) in [
        (
            "COMMAND",
            "scripted-executor".to_owned(),
            "command-consumer",
        ),
        (
            "REPORT",
            "workflow-application".to_owned(),
            "report-consumer",
        ),
        (
            "DECISION",
            format!("executor-{instance}"),
            "executor-decision-consumer",
        ),
        (
            "DECISION",
            "acceptance-harness".to_owned(),
            "acceptance-consumer",
        ),
    ] {
        let stream = format!("ORCHESTRATOR_WORKFLOW_{stream}_V1");
        let config = format!("--config=/config/{file}.json");
        if nats()?
            .args(["consumer", "info", &stream, &consumer])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?
            .success()
        {
            checked(nats()?.args(["consumer", "edit", &stream, &consumer, &config, "--force"]))?;
        } else {
            checked(nats()?.args(["consumer", "add", &stream, &config]))?;
        }
    }
    Ok(())
}
fn postgres_volume() -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let root = Path::new("/var/lib/postgresql/data");
    let metadata = fs::symlink_metadata(root)?;
    if !metadata.is_dir() {
        return Err("PostgreSQL volume must be a directory".into());
    }
    if metadata.uid() != 999 || metadata.gid() != 999 {
        own_tree(root)?;
    }
    fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
    Ok(())
}
fn own_tree(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    std::os::unix::fs::lchown(path, Some(999), Some(999))?;
    if metadata.is_dir() {
        for child in fs::read_dir(path)? {
            own_tree(&child?.path())?;
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nats_values_are_opaque_strings() {
        for text in ["123", "true", "$VALUE", "café", "", "123abc"] {
            assert_eq!(quote(text), format!("\"{text}\""));
        }
        assert_eq!(quote("a\"\\\n\t\u{7f}"), "\"a\\\"\\\\\\x0a\\x09\\x7f\"");
    }
}
