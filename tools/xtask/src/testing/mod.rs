mod loopback;
mod postgres;

use anyhow::{Result, bail};

pub fn run(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("postgres") => postgres::run(args.get(1).map(String::as_str)),
        Some("loopback") => loopback::run(),
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
