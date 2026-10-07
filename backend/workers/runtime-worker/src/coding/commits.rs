//! Coherent local milestones and fail-closed task constraint checks.
use crate::operations::runtime::git;
use ayni_orchestrator_profile_ayni::coding::WorkerSpec;
use std::{io, path::Path};

pub fn changed_paths(spec: &WorkerSpec, workspace: &Path) -> io::Result<Vec<String>> {
    git(
        workspace,
        &[
            "merge-base",
            "--is-ancestor",
            &spec.execution.revision,
            "HEAD",
        ],
    )?;
    git(
        workspace,
        &["-c", "core.hooksPath=/dev/null", "add", "--all", "--", "."],
    )?;
    git(
        workspace,
        &["reset", "--quiet", "HEAD", "--", ".ayni", ".orchestrator"],
    )?;
    let paths: Vec<String> = git(
        workspace,
        &[
            "diff",
            "--cached",
            "--name-only",
            "--no-renames",
            "-z",
            &spec.execution.revision,
        ],
    )?
    .split('\0')
    .filter(|v| !v.is_empty())
    .map(str::to_owned)
    .collect();
    if paths.len() > 256 || paths.iter().map(String::len).sum::<usize>() > 32768 {
        return Err(io::Error::other("changed paths outside bounds"));
    }
    for path in &paths {
        if !ayni_orchestrator_domain::repository_path_is_valid(path)
            || spec.constraints.protecting_path(path).is_some()
            || matches!(path.as_str(), ".ayni.lock" | ".ayni.toml")
        {
            return Err(io::Error::other("task changes a protected path"));
        }
        if !spec.constraints.allow_test_edits
            && (test_path(path)
                || (Path::new(path)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("rs"))
                    && rust_test_file_changed(workspace, path, &spec.execution.revision)))
        {
            return Err(io::Error::other("task test edits are not authorized"));
        }
    }
    Ok(paths)
}

pub fn conventional_message(message: &str) -> bool {
    if message.len() > 256 || message.chars().any(char::is_control) {
        return false;
    }
    let Some((prefix, description)) = message.split_once(": ") else {
        return false;
    };
    let prefix = prefix.strip_suffix('!').unwrap_or(prefix);
    let kind = if let Some((kind, scope)) = prefix.split_once('(') {
        if !scope.ends_with(')')
            || scope.len() < 2
            || !scope[..scope.len() - 1]
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'/'))
        {
            return false;
        }
        kind
    } else {
        prefix
    };
    !description.trim().is_empty()
        && matches!(
            kind,
            "feat"
                | "fix"
                | "refactor"
                | "docs"
                | "test"
                | "chore"
                | "build"
                | "ci"
                | "perf"
                | "style"
                | "revert"
        )
}

pub fn commit_changes(
    spec: &WorkerSpec,
    workspace: &Path,
    message: &str,
) -> io::Result<Option<String>> {
    if !conventional_message(message) {
        return Err(io::Error::other(
            "a Conventional Commit subject is required",
        ));
    }
    changed_paths(spec, workspace)?;
    if git(workspace, &["diff", "--cached", "--name-only", "HEAD"])?.is_empty() {
        return Ok(None);
    }
    git(
        workspace,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.name=Ayni Agent",
            "-c",
            "user.email=agent@invalid.example",
            "commit",
            "--quiet",
            "-s",
            "-m",
            message,
        ],
    )?;
    Ok(Some(git(workspace, &["rev-parse", "HEAD"])?))
}

fn test_path(path: &str) -> bool {
    path.split('/')
        .any(|part| matches!(part, "test" | "tests" | "__tests__"))
        || path.contains(".test.")
        || path.contains(".spec.")
        || path.ends_with("_test.go")
        || path.starts_with("test_")
}

fn rust_test_file_changed(workspace: &Path, path: &str, base: &str) -> bool {
    // Conservative until a language-specific syntax-aware policy is selected:
    // files containing inline tests require explicit test-edit permission.
    let Ok(old_paths) = git(workspace, &["ls-tree", "--name-only", base, "--", path]) else {
        return true;
    };
    let Ok(new_paths) = git(workspace, &["ls-files", "--", path]) else {
        return true;
    };
    let old = if old_paths.is_empty() {
        Ok(String::new())
    } else {
        git(workspace, &["show", &format!("{base}:{path}")])
    };
    let new = if new_paths.is_empty() {
        Ok(String::new())
    } else {
        git(workspace, &["show", &format!(":{path}")])
    };
    let (Ok(old), Ok(new)) = (old, new) else {
        return true;
    };
    old != new
        && [old, new]
            .iter()
            .any(|text| text.contains("#[test]") || text.contains("cfg(test)"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn milestone_messages_require_a_real_conventional_subject() {
        assert!(conventional_message(
            "feat(worker): bind final quality evidence"
        ));
        assert!(conventional_message("fix!: repair publication"));
        for message in [
            "done",
            "feat: ",
            "feat(): empty",
            "feat: unsafe\nsubject",
            "unknown: changes",
        ] {
            assert!(!conventional_message(message));
        }
    }
}
