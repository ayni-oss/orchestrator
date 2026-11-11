mod charts;
mod docs;
mod protocol;
mod security;
mod tooling;

use crate::{command, generate};
use anyhow::{Result, bail, ensure};

pub fn run(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("docs") => docs::links(),
        Some("docs-site") => docs::site(),
        Some("security") => security::check(),
        Some("tooling") => tooling::check(),
        Some("api") => protocol::check(),
        Some("charts") => charts::check(),
        Some("protocol") => {
            command::run("buf", &["format", "--diff", "--exit-code", "proto"])?;
            command::run("buf", &["lint", "proto"])?;
            if let Some(base) = args.get(1) {
                command::run("buf", &["breaking", "proto", "--against", base])?;
            }
            let before = generate::tree_bytes("clients/typescript/src/generated")?;
            generate::run(&["protocol".into()])?;
            ensure!(
                before == generate::tree_bytes("clients/typescript/src/generated")?,
                "protocol bindings differ; regenerate and commit them"
            );
            command::run(
                "pnpm",
                &["--filter", "@ayni-orchestrator/grpc-bindings", "check"],
            )?;
            command::run(
                "pnpm",
                &["--filter", "@ayni-orchestrator/grpc-bindings", "test"],
            )
        }
        _ => bail!("unknown check; use docs, docs-site, security, protocol or tooling"),
    }
}
