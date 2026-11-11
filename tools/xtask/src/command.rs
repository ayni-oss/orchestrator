use anyhow::{Context, Result, ensure};
use std::{
    ffi::OsStr,
    io::Write,
    process::{Command, Stdio},
};

pub fn output(program: &str, args: &[impl AsRef<OsStr>]) -> Result<String> {
    let result = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("start {program}"))?;
    ensure!(
        result.status.success(),
        "{program} failed ({})\n{}",
        result.status,
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).context("command output is not UTF-8")
}

pub fn run(program: &str, args: &[impl AsRef<OsStr>]) -> Result<()> {
    eprintln!("Running {program}");
    let status = Command::new(program)
        .args(args)
        .status()
        .with_context(|| format!("start {program}"))?;
    ensure!(status.success(), "{program} failed ({status})");
    Ok(())
}

pub fn input(program: &str, args: &[impl AsRef<OsStr>], bytes: &[u8]) -> Result<Vec<u8>> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().context("child stdin unavailable")?;
    // A separate writer avoids blocking when a child produces output while consuming input.
    let owned = bytes.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&owned));
    let result = child.wait_with_output()?;
    writer
        .join()
        .map_err(|_| anyhow::anyhow!("stdin writer panicked"))??;
    ensure!(
        result.status.success(),
        "{program} failed ({})",
        result.status
    );
    Ok(result.stdout)
}

pub fn files() -> Result<Vec<String>> {
    let mut paths: Vec<_> = output(
        "git",
        &["ls-files", "--cached", "--others", "--exclude-standard"],
    )?
    .lines()
    .map(str::to_owned)
    .collect();
    paths.sort();
    paths.dedup();
    Ok(paths)
}
