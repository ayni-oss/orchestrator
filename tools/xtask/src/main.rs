mod checks;
mod command;
mod dev;
mod evidence;
mod fixtures;
mod generate;
mod install;
mod testing;

use anyhow::{Result, bail};
use std::{env, path::PathBuf};

fn main() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    env::set_current_dir(&root)?;
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("dev") => dev::run(&args[1..]),
        Some("install") => install::run(
            args.get(1)
                .ok_or_else(|| anyhow::anyhow!("install requires helm or ayni"))?,
        ),
        Some("check") => checks::run(&args[1..]),
        Some("generate") => generate::run(&args[1..]),
        Some("evidence") => evidence::run(&args[1..]),
        Some("test") => testing::run(&args[1..]),
        None | Some("help" | "--help" | "-h") => {
            println!(
                "cargo xtask dev [status|stop|redeploy]\ncargo xtask check <docs|docs-site|security|protocol|api|charts|tooling>\ncargo xtask test <unit|frontend|postgres|jetstream|durability|loopback|deployment> [profile]\ncargo xtask generate <protocol|schema|fixtures|contract-bundle> [arguments]\ncargo xtask evidence <manifest|publication|recovery> [arguments]"
            );
            Ok(())
        }
        Some(other) => bail!("unknown xtask command: {other}"),
    }
}
