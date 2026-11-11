//! Deterministic workload inputs; fixture commits are deliberately unsigned synthetic data.
use anyhow::{Context as _, Result, ensure};
use sha2::{Digest as _, Sha256};
use std::{
    fs,
    io::Write as _,
    path::Path,
    process::{Command, Stdio},
};

pub fn generate() -> Result<()> {
    for quality in [false, true] {
        let root = Path::new(if quality {
            "dev/fixtures/task-quality"
        } else {
            "dev/fixtures/task-source"
        });
        let directory = tempfile::tempdir()?;
        let checkout = directory.path();
        if quality {
            for (name, bytes) in crate::generate::tree_bytes("dev/fixtures/task-quality/source")? {
                if Path::new(&name)
                    .components()
                    .any(|part| part.as_os_str() == ".ayni")
                {
                    continue;
                }
                let path = checkout.join(name);
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(path, bytes)?;
            }
        } else {
            fs::write(
                checkout.join(".ayni.lock"),
                "Acceptance fixture lock. Not an Ayni runtime compatibility claim.\n",
            )?;
            fs::write(checkout.join("example.txt"), "Original source.\n")?;
        }
        git(checkout, &["init", "--initial-branch=main"], None)?;
        git(checkout, &["add", "."], None)?;
        let message = if quality {
            "test: freeze managed task quality fixture"
        } else {
            "test: freeze task source fixture"
        };
        git(
            checkout,
            &[
                "-c",
                "user.name=Task Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-m",
                message,
            ],
            None,
        )?;
        let commit = String::from_utf8(git(checkout, &["rev-parse", "HEAD"], None)?)?;
        let objects = git(
            checkout,
            &["rev-list", "--objects", "--no-object-names", "HEAD"],
            None,
        )?;
        let pack = git(checkout, &["pack-objects", "--stdout"], Some(&objects))?;
        let lock = fs::read(checkout.join(".ayni.lock"))?;
        let metadata = serde_json::json!({"source_commit":commit.trim(),"lock_digest":format!("sha256:{:x}",Sha256::digest(lock)),"source_pack_digest":format!("sha256:{:x}",Sha256::digest(&pack))});
        fs::write(root.join("source.pack"), pack)?;
        fs::write(
            root.join("source.json"),
            format!("{}\n", serde_json::to_string_pretty(&metadata)?),
        )?;
    }
    Ok(())
}
fn git(checkout: &Path, args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>> {
    let mut child = Command::new("git")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgsign=false",
        ])
        .arg("-C")
        .arg(checkout)
        .args(args)
        .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(bytes) = input {
        child
            .stdin
            .take()
            .context("fixture git stdin missing")?
            .write_all(bytes)?;
    }
    let output = child.wait_with_output()?;
    ensure!(
        output.status.success(),
        "fixture git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(output.stdout)
}
