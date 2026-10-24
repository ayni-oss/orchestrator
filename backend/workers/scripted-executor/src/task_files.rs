//! Bounded file changes, with no shell tools and no symlink traversal.
use ayni_orchestrator_events::task_execution::{MAX_PATCH, TaskCommand, digest};
use rustix::fs::{Mode, OFlags, mkdirat, open, openat, renameat};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read as _, Write as _},
    os::fd::OwnedFd,
    path::{Component, Path},
    process::{Command, Stdio},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileChange {
    pub path: String,
    pub content: String,
}
/// Operate on a bounded task-owned source snapshot.
/// # Errors
/// Rejects invalid paths, unavailable objects, symlinks or exceeded bounds.
pub fn git(root: &Path, args: &[&str], input: Option<&[u8]>) -> io::Result<Vec<u8>> {
    let mut c = Command::new("git");
    c.current_dir(root)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "protocol.file.allow=never",
        ])
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = c.spawn()?;
    if let Some(bytes) = input {
        child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("git input unavailable"))?
            .write_all(bytes)?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Err(io::Error::other("task source operation failed"));
    }
    Ok(out.stdout)
}
/// Operate on a bounded task-owned source snapshot.
/// # Errors
/// Rejects invalid paths, unavailable objects, symlinks or exceeded bounds.
pub fn checkout(root: &Path, pack: &[u8], command: &TaskCommand) -> io::Result<()> {
    if digest(pack) != command.source_pack_digest {
        return Err(io::Error::other("source pack identity mismatch"));
    }
    fs::create_dir_all(root)?;
    git(root, &["init", "--initial-branch=task"], None)?;
    git(root, &["unpack-objects", "-q"], Some(pack))?;
    let entries = git(
        root,
        &["ls-tree", "-r", "--full-tree", &command.source_commit],
        None,
    )?;
    if entries
        .split(|b| *b == b'\n')
        .any(|line| line.starts_with(b"160000 "))
    {
        return Err(io::Error::other("task submodules unsupported"));
    }
    git(
        root,
        &[
            "-c",
            "core.symlinks=true",
            "checkout",
            "--force",
            "--detach",
            &command.source_commit,
        ],
        None,
    )?;
    let lock = read_file(root, ".ayni.lock", 4 * 1024 * 1024)?;
    if digest(lock.as_bytes()) != command.lock_digest {
        return Err(io::Error::other("source lock identity mismatch"));
    }
    Ok(())
}
/// Reconstruct coding inputs with an approved retained patch before any provider permit.
/// # Errors
/// Rejects changed patch identities, protected paths, symlinks and invalid Git patches.
pub fn checkout_for_coding(root: &Path, pack: &[u8], command: &TaskCommand) -> io::Result<()> {
    checkout(root, pack, command)?;
    let Some(recovery) = &command.recovery else {
        return Ok(());
    };
    if recovery.seed_patch.len() > MAX_PATCH
        || digest(recovery.seed_patch.as_bytes()) != recovery.seed_patch_digest
    {
        return Err(io::Error::other("saved patch identity mismatch"));
    }
    git(
        root,
        &[
            "apply",
            "--allow-empty",
            "--index",
            "--whitespace=nowarn",
            "-",
        ],
        Some(recovery.seed_patch.as_bytes()),
    )?;
    let names = git(
        root,
        &["diff", "--name-only", "--no-renames", "-z", "HEAD", "--"],
        None,
    )?;
    for name in names.split(|b| *b == 0).filter(|p| !p.is_empty()) {
        let path = std::str::from_utf8(name).map_err(|_| io::Error::other("saved path invalid"))?;
        parts(path)?;
        if matches!(path, ".ayni.lock" | ".ayni.toml") {
            return Err(io::Error::other("saved protected input"));
        }
        // No deletions, symlinks, binary content or unavailable file content may reach coding.
        read_file(root, path, 128 * 1024)?;
    }
    if digest(read_file(root, ".ayni.lock", 4 * 1024 * 1024)?.as_bytes()) != command.lock_digest {
        return Err(io::Error::other("saved lock changed"));
    }
    Ok(())
}
fn parts(path: &str) -> io::Result<Vec<&str>> {
    if path.len() > 256 || path.is_empty() {
        return Err(io::Error::other("task path invalid"));
    }
    let mut out = Vec::new();
    for part in Path::new(path).components() {
        match part {
            Component::Normal(p) => {
                let p = p
                    .to_str()
                    .ok_or_else(|| io::Error::other("task path invalid"))?;
                if p == ".git" || p == ".ayni" || p.chars().any(char::is_control) {
                    return Err(io::Error::other("task path protected"));
                }
                out.push(p);
            }
            _ => return Err(io::Error::other("task path invalid")),
        }
    }
    if out.is_empty() || out.join("/") != path {
        return Err(io::Error::other("task path invalid"));
    }
    Ok(out)
}
fn parent(root: &Path, parts: &[&str], create: bool) -> io::Result<OwnedFd> {
    let mut dir = open(
        root,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW,
        Mode::empty(),
    )?;
    for part in parts {
        if create {
            match mkdirat(&dir, *part, Mode::from_bits_truncate(0o755)) {
                Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                Err(e) => return Err(e.into()),
            }
        }
        dir = openat(
            &dir,
            *part,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW,
            Mode::empty(),
        )?;
    }
    Ok(dir)
}
/// Operate on a bounded task-owned source snapshot.
/// # Errors
/// Rejects invalid paths, unavailable objects, symlinks or exceeded bounds.
pub fn read_file(root: &Path, path: &str, maximum: usize) -> io::Result<String> {
    let parts = parts(path)?;
    let (leaf, parents) = parts
        .split_last()
        .ok_or_else(|| io::Error::other("task path invalid"))?;
    let dir = parent(root, parents, false)?;
    let fd = openat(
        &dir,
        *leaf,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    )?;
    let file = fs::File::from(fd);
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("task source must be regular"));
    }
    let mut bytes = Vec::new();
    file.take(u64::try_from(maximum).unwrap_or(u64::MAX) + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > maximum {
        return Err(io::Error::other("source context limit"));
    }
    String::from_utf8(bytes).map_err(|_| io::Error::other("task source must be UTF-8"))
}
/// Operate on a bounded task-owned source snapshot.
/// # Errors
/// Rejects invalid paths, unavailable objects, symlinks or exceeded bounds.
pub fn context(root: &Path, commit: &str) -> io::Result<Vec<FileChange>> {
    let head = git(root, &["rev-parse", "HEAD"], None)?;
    if String::from_utf8_lossy(&head).trim() != commit {
        return Err(io::Error::other("source commit changed"));
    }
    let names = git(root, &["ls-files", "--cached", "-z"], None)?;
    let names = names
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>();
    if names.len() > 64 {
        return Err(io::Error::other("source context limit"));
    }
    let mut size = 0;
    let mut result = Vec::new();
    for name in names {
        let path = std::str::from_utf8(name).map_err(|_| io::Error::other("task path invalid"))?;
        let text = read_file(root, path, 128 * 1024)?;
        size += text.len();
        if size > 128 * 1024 {
            return Err(io::Error::other("source context limit"));
        }
        result.push(FileChange {
            path: path.to_owned(),
            content: text,
        });
    }
    Ok(result)
}
/// Operate on a bounded task-owned source snapshot.
/// # Errors
/// Rejects invalid paths, unavailable objects, symlinks or exceeded bounds.
pub fn apply(root: &Path, changes: &[FileChange]) -> io::Result<String> {
    if changes.is_empty()
        || changes.len() > 16
        || changes.iter().map(|v| v.content.len()).sum::<usize>() > 32 * 1024
    {
        return Err(io::Error::other("task changes exceed bounds"));
    }
    let mut seen = std::collections::BTreeSet::new();
    // Validate content bounds and lexical paths before writing file contents.
    for change in changes {
        parts(&change.path)?;
        if !seen.insert(&change.path) || matches!(change.path.as_str(), ".ayni.lock" | ".ayni.toml")
        {
            return Err(io::Error::other("task changes invalid"));
        }
    }
    for change in changes {
        let parts = parts(&change.path)?;
        let (leaf, parents) = parts
            .split_last()
            .ok_or_else(|| io::Error::other("task path invalid"))?;
        let dir = parent(root, parents, true)?;
        let name = format!(".ayni-task-write-{}", uuid::Uuid::now_v7());
        let fd = openat(
            &dir,
            name.as_str(),
            OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY | OFlags::NOFOLLOW,
            Mode::from_bits_truncate(0o644),
        )?;
        let mut file = fs::File::from(fd);
        file.write_all(change.content.as_bytes())?;
        file.sync_all()?;
        renameat(&dir, name.as_str(), &dir, *leaf)?;
        git(root, &["add", "--intent-to-add", "--", &change.path], None)?;
    }
    let patch = git(
        root,
        &[
            "diff",
            "--binary",
            "--no-ext-diff",
            "--no-textconv",
            "HEAD",
            "--",
        ],
        None,
    )?;
    if patch.len() > MAX_PATCH {
        return Err(io::Error::other("task patch limit"));
    }
    String::from_utf8(patch).map_err(|_| io::Error::other("task patch invalid"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_tools_refuse_escape_and_symlink_parents() -> io::Result<()> {
        let source = tempfile::tempdir()?;
        let outside = tempfile::tempdir()?;
        std::os::unix::fs::symlink(outside.path(), source.path().join("escape"))?;
        for path in ["../outside", "/absolute", ".git/config", "escape/file"] {
            assert!(
                apply(
                    source.path(),
                    &[FileChange {
                        path: path.into(),
                        content: "denied".into()
                    }]
                )
                .is_err()
            );
        }
        assert!(!outside.path().join("file").exists());
        Ok(())
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    #[test]
    fn exact_git_snapshot_and_file_proposals_survive_replay() -> io::Result<()> {
        let original = tempfile::tempdir()?;
        let target = tempfile::tempdir()?;
        git(original.path(), &["init", "--initial-branch=main"], None)?;
        fs::write(original.path().join(".ayni.lock"), "fixture lock\n")?;
        fs::write(original.path().join("file.txt"), "before\n")?;
        git(original.path(), &["add", "."], None)?;
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
                "fixture",
            ],
            None,
        )?;
        let commit = String::from_utf8(git(original.path(), &["rev-parse", "HEAD"], None)?)
            .map_err(io::Error::other)?
            .trim()
            .to_owned();
        let ids = git(
            original.path(),
            &["rev-list", "--objects", "--no-object-names", "HEAD"],
            None,
        )?;
        let pack = git(original.path(), &["pack-objects", "--stdout"], Some(&ids))?;
        let command = TaskCommand {
            source_commit: commit,
            source_pack_digest: digest(&pack),
            lock_digest: digest(b"fixture lock\n"),
            ..Default::default()
        };
        checkout(target.path(), &pack, &command)?;
        let changes = vec![
            FileChange {
                path: "file.txt".into(),
                content: "after\n".into(),
            },
            FileChange {
                path: "nested/new.txt".into(),
                content: "new\n".into(),
            },
        ];
        let patch = apply(target.path(), &changes)?;
        assert!(patch.contains("+after"));
        assert!(patch.contains("nested/new.txt"));
        assert_eq!(apply(target.path(), &changes)?, patch);
        let mut continued = command.clone();
        continued.recovery = Some(
            ayni_orchestrator_events::task_execution::TaskRecoveryCommand {
                seed_patch: patch.clone(),
                seed_patch_digest: digest(patch.as_bytes()),
                ..Default::default()
            },
        );
        let next = tempfile::tempdir()?;
        checkout_for_coding(next.path(), &pack, &continued)?;
        let context = context(next.path(), &command.source_commit)?;
        assert!(
            context
                .iter()
                .any(|f| f.path == "nested/new.txt" && f.content == "new\n")
        );
        assert_eq!(read_file(next.path(), "file.txt", 128)?, "after\n");
        let final_patch = apply(
            next.path(),
            &[FileChange {
                path: "next.txt".into(),
                content: "continued\n".into(),
            }],
        )?;
        assert!(final_patch.contains("+after"));
        assert!(final_patch.contains("nested/new.txt"));
        assert!(final_patch.contains("next.txt"));
        continued
            .recovery
            .as_mut()
            .ok_or_else(|| io::Error::other("recovery missing"))?
            .seed_patch_digest = digest(b"wrong");
        assert!(checkout_for_coding(tempfile::tempdir()?.path(), &pack, &continued).is_err());

        assert!(checkout(tempfile::tempdir()?.path(), b"corrupt", &command).is_err());
        Ok(())
    }
}
