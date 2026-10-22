//! Read trusted local Git objects; never fetch, run hooks or interpret lock policy.
use sha2::{Digest as _, Sha256};
use std::{error::Error, io, path::Path, process::Command};
type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub fn verify(checkout: &Path, repository: &str, source: &str) -> Result<(String, String)> {
    if repository.is_empty() || source.is_empty() || source.starts_with('-') {
        return Err(io::Error::other("approved repository and source are required").into());
    }
    let remote = git(checkout, &["remote", "get-url", "origin"])?;
    if remote.trim() != repository {
        return Err(
            io::Error::other("checkout origin differs from the approved repository").into(),
        );
    }
    let commit = git(
        checkout,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{source}^{{commit}}"),
        ],
    )?
    .trim()
    .to_owned();
    if commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(io::Error::other("source must resolve to a SHA-1 Git commit").into());
    }
    let object = format!("{commit}:.ayni.lock");
    let size: usize = git(checkout, &["cat-file", "-s", &object])?
        .trim()
        .parse()?;
    if size == 0 || size > 4 * 1024 * 1024 {
        return Err(io::Error::other("frozen Ayni lock is empty or exceeds 4 MiB").into());
    }
    // cat-file reads the exact blob, ignoring dirty files, attributes and filters.
    let lock = command(checkout)
        .args(["cat-file", "blob", &object])
        .output()?;
    if !lock.status.success() || lock.stdout.len() != size {
        return Err(io::Error::other("cannot read frozen Ayni lock blob").into());
    }
    Ok((commit, format!("sha256:{:x}", Sha256::digest(&lock.stdout))))
}
fn command(checkout: &Path) -> Command {
    let mut command = Command::new("git");
    command.current_dir(checkout).args([
        "-c",
        "core.fsmonitor=false",
        "-c",
        "core.hooksPath=/dev/null",
    ]);
    command
}
fn git(checkout: &Path, args: &[&str]) -> Result<String> {
    let output = command(checkout).args(args).output()?;
    if !output.status.success() {
        // Git stderr can contain credential-bearing remote URLs; do not expose it.
        return Err(io::Error::other("local Git evidence verification failed").into());
    }
    Ok(String::from_utf8(output.stdout)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn freezes_committed_lock_and_rejects_wrong_repository() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path();
        git(path, &["init", "--initial-branch=main"])?;
        git(
            path,
            &[
                "remote",
                "add",
                "origin",
                "https://example.invalid/repo.git",
            ],
        )?;
        std::fs::write(path.join(".ayni.lock"), "committed lock\n")?;
        git(path, &["add", ".ayni.lock"])?;
        git(
            path,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "fixture",
            ],
        )?;
        let first = verify(path, "https://example.invalid/repo.git", "refs/heads/main")?;
        std::fs::write(path.join(".ayni.lock"), "uncommitted replacement")?;
        assert_eq!(
            verify(path, "https://example.invalid/repo.git", "refs/heads/main")?,
            first
        );
        assert_eq!(
            first.1,
            format!("sha256:{:x}", Sha256::digest(b"committed lock\n"))
        );
        assert!(verify(path, "https://example.invalid/other.git", "HEAD").is_err());
        assert!(verify(path, "https://example.invalid/repo.git", "--help").is_err());
        assert!(verify(path, "https://example.invalid/repo.git", "missing").is_err());
        Ok(())
    }
}
