mod deployment;
mod jetstream;
mod loopback;
mod postgres;
mod topology;

use anyhow::{Result, bail};

pub fn run(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("postgres") => postgres::run(args.get(1).map(String::as_str)),
        Some("loopback") => loopback::run(),
        Some("jetstream") => jetstream::run(false),
        Some("durability") => jetstream::run(true),
        Some("deployment") => deployment::run(args.get(1).map(String::as_str)),
        Some("retry-backoff") => retry_backoff(),
        Some("unit") => crate::command::run(
            "cargo",
            &["test", "--locked", "--workspace", "--all-targets"],
        ),
        Some("frontend") => {
            crate::command::run("pnpm", &["--filter", "@ayni-orchestrator/web", "test"])
        }
        _ => bail!("unknown test suite"),
    }
}

fn retry_backoff() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let binary = directory.path().join("retry-backoff");
    anyhow::ensure!(
        std::process::Command::new("rustc")
            .arg("examples/retry-backoff/main.rs")
            .arg("-o")
            .arg(&binary)
            .status()?
            .success(),
        "retry-backoff fixture compilation failed"
    );
    for (attempt, expected) in [(1, 2), (2, 4), (3, 8), (6, 60)] {
        let output = std::process::Command::new(&binary)
            .arg(attempt.to_string())
            .output()?;
        anyhow::ensure!(output.status.success(), "attempt {attempt} failed");
        let actual = String::from_utf8(output.stdout)?;
        anyhow::ensure!(
            actual.trim() == expected.to_string(),
            "attempt {attempt}: expected {expected} seconds, got {}",
            actual.trim()
        );
    }
    println!("retry-backoff formula matches all four empirical cases");
    Ok(())
}
