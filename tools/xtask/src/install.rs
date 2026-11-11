use anyhow::{Context as _, Result, ensure};
use sha2::{Digest as _, Sha256};
use std::{
    env, fs,
    io::{Read as _, Write as _},
    os::unix::fs::PermissionsExt,
    path::Path,
};

/// CI bootstrap uses the same immutable pins as development. No shell implementation.
pub fn run(name: &str) -> Result<()> {
    ensure!(
        env::consts::OS == "linux" && env::consts::ARCH == "x86_64",
        "CI bootstrap requires linux/amd64"
    );
    let scratch = tempfile::tempdir()?;
    let pins: serde_json::Value = serde_json::from_slice(&fs::read("tools/xtask/ayni-tool.json")?)?;
    let version = pins["cli_version"]
        .as_str()
        .context("CLI version missing")?;
    let (url, checksum, archive_path) = match name {
        "helm" => (
            "https://get.helm.sh/helm-v4.1.1-linux-amd64.tar.gz".into(),
            "5d4c7623283e6dfb1971957f4b755468ab64917066a8567dd50464af298f4031",
            "linux-amd64/helm".into(),
        ),
        "ayni" => (
            format!(
                "https://github.com/ayni-oss/ayni/releases/download/ayni-v{version}/ayni-ayni-v{version}-x86_64-unknown-linux-gnu.tar.gz"
            ),
            pins["linux_amd64_archive_sha256"]
                .as_str()
                .context("CLI checksum missing")?,
            format!("ayni-ayni-v{version}-x86_64-unknown-linux-gnu/ayni"),
        ),
        _ => anyhow::bail!("unknown pinned tool: {name}"),
    };
    let archive = scratch.path().join("archive.tar.gz");
    crate::command::run(
        "curl",
        &[
            "--fail",
            "--location",
            "--proto",
            "=https",
            "--tlsv1.2",
            "--retry",
            "3",
            "--output",
            archive.to_str().context("non-UTF8 temporary path")?,
            &url,
        ],
    )?;
    let bytes = fs::read(archive)?;
    ensure!(
        format!("{:x}", Sha256::digest(&bytes)) == checksum,
        "pinned tool checksum mismatch"
    );
    let bin = std::path::PathBuf::from(env::var_os("RUNNER_TEMP").context("RUNNER_TEMP missing")?)
        .join(format!("{name}-bin"));
    fs::create_dir_all(&bin)?;
    let destination = bin.join(name);
    let mut found = false;
    for entry in tar::Archive::new(flate2::read::GzDecoder::new(bytes.as_slice())).entries()? {
        let mut entry = entry?;
        if entry.path()?.as_ref() == Path::new(&archive_path)
            && entry.header().entry_type().is_file()
        {
            let mut executable = Vec::new();
            entry.read_to_end(&mut executable)?;
            fs::write(&destination, executable)?;
            fs::set_permissions(&destination, fs::Permissions::from_mode(0o755))?;
            found = true;
        }
    }
    ensure!(found, "pinned archive executable missing");
    append("GITHUB_PATH", &bin.display().to_string())?;
    if name == "ayni" {
        append("GITHUB_OUTPUT", &format!("cli={}", destination.display()))?;
    }
    Ok(())
}
fn append(variable: &str, value: &str) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(env::var_os(variable).with_context(|| format!("{variable} missing"))?)?;
    writeln!(file, "{value}")?;
    Ok(())
}
