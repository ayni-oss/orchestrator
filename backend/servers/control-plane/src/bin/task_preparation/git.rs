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

/// Verify frozen upstream commits are ancestors of the explicitly selected
/// integrated source. Reads local objects only; it never merges, fetches or
/// changes a branch. The operator resolves merge conflicts before preparation.
pub fn verify_upstreams(checkout: &Path, source: &str, commits: &[String]) -> Result<()> {
    let oid = |value: &str| {
        value.len() == 40
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    };
    if !oid(source) || commits.len() > 31 || commits.iter().any(|value| !oid(value)) {
        return Err(io::Error::other("frozen upstream commit invalid").into());
    }
    for commit in commits {
        let status = command(checkout)
            .args(["merge-base", "--is-ancestor", commit, source])
            .output()?;
        if !status.status.success() {
            return Err(io::Error::other(
                "selected source does not contain every frozen upstream commit",
            )
            .into());
        }
    }
    Ok(())
}

/// Export only the frozen commit and its reachable snapshot objects, excluding history.
pub fn snapshot_pack(checkout: &Path, commit: &str) -> Result<Vec<u8>> {
    use std::{
        collections::BTreeSet,
        io::{Read as _, Write as _},
        process::Stdio,
    };
    if commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(io::Error::other("frozen commit invalid").into());
    }
    let tree = git(checkout, &["rev-parse", &format!("{commit}^{{tree}}")])?;
    let output = command(checkout)
        .args(["ls-tree", "-r", "-t", "-z", commit])
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other("frozen tree unavailable").into());
    }
    let mut ids = BTreeSet::from([commit.to_owned(), tree.trim().to_owned()]);
    for entry in output.stdout.split(|b| *b == 0).filter(|b| !b.is_empty()) {
        let header = entry
            .split(|b| *b == b'\t')
            .next()
            .ok_or("tree entry invalid")?;
        let header = std::str::from_utf8(header)?;
        let fields = header.split(' ').collect::<Vec<_>>();
        if fields.len() != 3 || fields[0] == "160000" {
            return Err(io::Error::other("submodule snapshots unsupported").into());
        }
        ids.insert(fields[2].to_owned());
    }
    let mut child = command(checkout)
        .args(["pack-objects", "--stdout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let input = ids.into_iter().collect::<Vec<_>>().join("\n") + "\n";
    let mut stdin = child.stdin.take().ok_or("pack input missing")?;
    let producer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let mut pack = Vec::new();
    child
        .stdout
        .take()
        .ok_or("pack output missing")?
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut pack)?;
    if pack.len() > 8 * 1024 * 1024 {
        let _ = child.kill();
        let _ = child.wait();
        let _ = producer.join();
        return Err(io::Error::other("source pack exceeds 8 MiB").into());
    }
    let status = child.wait()?;
    producer.join().map_err(|_| "pack input failed")??;
    if !status.success() {
        return Err(io::Error::other("source pack failed").into());
    }
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn integrated_source_requires_every_selected_upstream_commit() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path();
        git(path, &["init", "--initial-branch=main"])?;
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
                "--allow-empty",
                "-m",
                "base",
            ],
        )?;
        let base = git(path, &["rev-parse", "HEAD"])?.trim().to_owned();
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
                "--allow-empty",
                "-m",
                "upstream",
            ],
        )?;
        let upstream = git(path, &["rev-parse", "HEAD"])?.trim().to_owned();
        verify_upstreams(path, &upstream, &[base.clone(), upstream.clone()])?;
        assert!(verify_upstreams(path, &base, &[upstream]).is_err());
        assert!(verify_upstreams(path, &base, &["--help".into()]).is_err());
        Ok(())
    }
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
    #[test]
    fn snapshot_pack_contains_the_frozen_tree_without_parent_history() -> Result<()> {
        use std::{io::Write as _, process::Stdio};
        let original = tempfile::tempdir()?;
        let target = tempfile::tempdir()?;
        git(original.path(), &["init", "--initial-branch=main"])?;
        for value in ["first", "second"] {
            std::fs::write(original.path().join("file.txt"), value)?;
            git(original.path(), &["add", "file.txt"])?;
            git(
                original.path(),
                &[
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "-c",
                    "commit.gpgsign=false",
                    "commit",
                    "-m",
                    value,
                ],
            )?;
        }
        let commit = git(original.path(), &["rev-parse", "HEAD"])?;
        let commit = commit.trim();
        let parent = git(original.path(), &["rev-parse", "HEAD^"])?;
        let pack = snapshot_pack(original.path(), commit)?;
        git(target.path(), &["init", "--initial-branch=task"])?;
        let mut child = command(target.path())
            .args(["unpack-objects", "-q"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        child
            .stdin
            .take()
            .ok_or("fixture stdin")?
            .write_all(&pack)?;
        assert!(child.wait()?.success());
        git(target.path(), &["checkout", "--detach", commit])?;
        assert_eq!(
            std::fs::read_to_string(target.path().join("file.txt"))?,
            "second"
        );
        assert!(git(target.path(), &["cat-file", "-e", parent.trim()]).is_err());
        Ok(())
    }
}
