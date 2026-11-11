use anyhow::Result;
use sha2::{Digest as _, Sha256};
use std::{collections::BTreeMap, fs, path::Path};

pub fn snapshot() -> Result<BTreeMap<String, String>> {
    snapshot_at(Path::new("."), &crate::command::files()?)
}

fn snapshot_at(root: &Path, files: &[String]) -> Result<BTreeMap<String, String>> {
    let mut hashes: BTreeMap<&str, Sha256> = [
        "control-plane",
        "conversation-supervisor",
        "runtime",
        "schema",
        "chart",
        "controller",
    ]
    .into_iter()
    .map(|name| (name, Sha256::new()))
    .collect();
    for path in files {
        let name = path.as_str();
        if !root.join(path).is_file() {
            continue;
        }
        let components = components(name);
        if components.is_empty() {
            continue;
        }
        let bytes = match fs::read(root.join(path)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        for component in components {
            if let Some(hash) = hashes.get_mut(component) {
                hash.update(name.as_bytes());
                hash.update([0]);
                hash.update(&bytes);
                hash.update([0]);
            }
        }
    }
    Ok(hashes
        .into_iter()
        .map(|(name, hash)| (name.into(), format!("{:x}", hash.finalize())))
        .collect())
}

pub fn freeze(expected: &BTreeMap<String, String>) -> Result<tempfile::TempDir> {
    let directory = tempfile::tempdir()?;
    let files = crate::command::files()?;
    for name in &files {
        let source = Path::new(name);
        if !source.is_file() {
            continue;
        }
        anyhow::ensure!(
            !source.is_symlink(),
            "build input must not point outside the frozen source: {name}"
        );
        let destination = directory.path().join(name);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, destination)?;
    }
    anyhow::ensure!(
        &snapshot_at(directory.path(), &files)? == expected && &snapshot()? == expected,
        "source changed while freezing build inputs; retrying is required"
    );
    Ok(directory)
}

fn components(name: &str) -> Vec<&'static str> {
    if name.starts_with("tools/") || name == ".cargo/config.toml" {
        return vec!["controller"];
    }
    if name.starts_with("dev/charts/") || name.starts_with("deploy/charts/") {
        return vec!["chart"];
    }
    let shared = [
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        ".dockerignore",
    ]
    .contains(&name)
        || name.starts_with("backend/libs/")
        || name.starts_with("proto/")
        || name == "dev/live.Dockerfile";
    if shared {
        return vec!["control-plane", "conversation-supervisor", "runtime"];
    }
    if name.starts_with("backend/servers/control-plane/") {
        let mut groups = vec!["control-plane", "conversation-supervisor", "runtime"];
        if name.contains("workflow-postgres/migrations/")
            || name.ends_with("workflow-postgres/src/migrations.rs")
        {
            groups.push("schema");
        }
        return groups;
    }
    if name.starts_with("backend/workers/conversation-supervisor/") {
        return vec!["conversation-supervisor"];
    }
    if name.starts_with("backend/workers/runtime-")
        || name.starts_with("deploy/images/runtime-")
        || name == "scripts/ci/ayni-tool.json"
    {
        return vec!["runtime"];
    }
    if name.starts_with("dev/fixtures/task-") {
        return vec!["control-plane", "conversation-supervisor", "runtime"];
    }
    if Path::new(name).extension().is_some_and(|ext| ext == "sql") {
        return vec!["schema"];
    }
    Vec::new()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reload_boundaries() {
        assert!(components("apps/web/src/routes/+page.svelte").is_empty());
        assert_eq!(
            components("backend/workers/conversation-supervisor/src/main.rs"),
            vec!["conversation-supervisor"]
        );
        assert!(
            components(
                "backend/servers/control-plane/crates/workflow-postgres/migrations/019_next.sql"
            )
            .contains(&"schema")
        );
        assert_eq!(components("tools/xtask/src/main.rs"), vec!["controller"]);
    }
}
