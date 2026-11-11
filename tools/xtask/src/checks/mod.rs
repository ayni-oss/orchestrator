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
        Some("contract-baseline") => {
            let current = std::fs::read_to_string("proto/BASELINE")?;
            let base = std::fs::read_to_string(".contract-base/proto/BASELINE")?;
            ensure!(
                current.trim() == "workflow-m1-private-v1",
                "canonical baseline changed"
            );
            ensure!(
                base == current || base.trim() == "repository-builders-v4",
                "incompatible contract baseline"
            );
            let mut args = vec!["protocol".into()];
            if base == current {
                args.push(".contract-base/proto".into());
            }
            run(&args)
        }
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
