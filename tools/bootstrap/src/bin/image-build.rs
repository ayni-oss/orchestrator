//! Docker build orchestration, compiled with rustc so bootstrap has no dependencies.
use std::{env, error::Error, fs, os::unix::fs::PermissionsExt, path::Path, process::Command};
type Result<T> = std::result::Result<T, Box<dyn Error>>;
fn run(command: &mut Command) -> Result<()> {
    if !command.status()?.success() {
        return Err("image build command failed".into());
    }
    Ok(())
}
fn main() -> Result<()> {
    fs::create_dir_all("/output")?;
    match env::args().nth(1).as_deref() {
        Some("live") => live(),
        Some("runtime") => runtime(),
        _ => Err("expected live or runtime".into()),
    }
}
fn live() -> Result<()> {
    let component = env::var("COMPONENT")?;
    let bins: &[&str] = match component.as_str() {
        "control-plane" => &[
            "ayni-orchestrator-control-plane",
            "ayni-log-collector",
            "workflow-migrate",
            "workflow-diagnostic",
            "workflow-acceptance",
            "workflow-publish-task",
            "workflow-log-fixture",
            "workflow-browser-fixture",
        ],
        "conversation-supervisor" => &[
            "ayni-orchestrator-conversation-supervisor",
            "ayni-task-worker",
            "ayni-task-supervisor",
            "ayni-task-publisher",
        ],
        _ => return Err("unsupported component".into()),
    };
    let mut cargo = Command::new("cargo");
    cargo
        .env("CARGO_PROFILE_DEV_DEBUG", "0")
        .env("CARGO_INCREMENTAL", "1")
        .args([
            "build",
            "--locked",
            "-p",
            &format!("ayni-orchestrator-{component}"),
        ]);
    for bin in bins {
        cargo.args(["--bin", bin]);
    }
    run(&mut cargo)?;
    for bin in bins {
        copy(&format!("target/debug/{bin}"), bin)?;
    }
    if component == "control-plane" {
        run(Command::new("rustc").args([
            "--edition=2024",
            "-C",
            "target-feature=+crt-static",
            "-O",
            "tools/bootstrap/src/bin/local-infrastructure.rs",
            "-o",
            "/output/ayni-local-infrastructure",
        ]))?;
    }
    Ok(())
}
fn runtime() -> Result<()> {
    let output = Command::new("rustc").arg("-vV").output()?;
    let version = String::from_utf8(output.stdout)?;
    let target = version
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .ok_or("missing compiler target")?;
    // Source snapshots have fresh mtimes; Cargo fingerprints include their contents.
    let mut cargo = Command::new("cargo");
    cargo
        .env("CARGO_BUILD_JOBS", "2")
        .env("CARGO_INCREMENTAL", "0")
        .env("CARGO_PROFILE_RELEASE_DEBUG", "0")
        .env("CARGO_PROFILE_RELEASE_LTO", "false")
        .env("RUSTFLAGS", "-C target-feature=+crt-static")
        .args([
            "build",
            "--locked",
            "--release",
            "--target",
            target,
            "--package",
            "ayni-orchestrator-runtime-preparation",
            "--package",
            "ayni-orchestrator-runtime-verifier",
        ]);
    run(&mut cargo)?;
    for (bin, name) in [
        ("ayni-orchestrator-runtime-preparation", "runtime-worker"),
        ("ayni-runtime-supervisor", "runtime-supervisor"),
        ("ayni-orchestrator-runtime-verifier", "runtime-verifier"),
    ] {
        let source = format!("target/{target}/release/{bin}");
        for (option, forbidden) in [("--program-headers", "INTERP"), ("--dynamic", "(NEEDED)")] {
            let output = Command::new("readelf").args([option, &source]).output()?;
            if !output.status.success()
                || String::from_utf8_lossy(&output.stdout).contains(forbidden)
            {
                return Err("runtime binary must be static".into());
            }
        }
        copy(&source, name)?;
    }
    if env::var("KEEP_TARGET_CACHE").as_deref() != Ok("true") {
        for entry in fs::read_dir("target")? {
            let path = entry?.path();
            if path.is_dir() {
                fs::remove_dir_all(path)?;
            } else {
                fs::remove_file(path)?;
            }
        }
    }
    Ok(())
}
fn copy(source: &str, name: &str) -> Result<()> {
    let destination = Path::new("/output").join(name);
    fs::copy(source, &destination)?;
    fs::set_permissions(destination, fs::Permissions::from_mode(0o555))?;
    Ok(())
}
