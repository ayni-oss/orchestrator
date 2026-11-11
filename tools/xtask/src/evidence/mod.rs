mod publication;
mod recovery;
mod traces;

use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path},
};

pub fn run(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("traces") => traces::run(&args[1..])?,
        Some("manifest") => {
            let root = Path::new(
                args.get(1)
                    .context("manifest requires evidence directory")?,
            );
            let report = json(root, args.get(2).map_or("outcome.json", String::as_str))?;
            verify_manifest(root, &report["artifacts_sha256"], &[])?;
        }
        Some("publication") => {
            ensure!(
                args.len() == 5 || args.len() == 6,
                "publication BASELINE CURRENT CREATES PULLS [IDENTITY]"
            );
            let baseline = serde_json::from_slice(&fs::read(&args[1])?)?;
            let current = serde_json::from_slice(&fs::read(&args[2])?)?;
            let identity = args
                .get(5)
                .map(fs::read)
                .transpose()?
                .map(|v| serde_json::from_slice(&v))
                .transpose()?;
            println!(
                "{}",
                publication::verify(
                    &baseline,
                    &current,
                    args[3].parse()?,
                    args[4].parse()?,
                    identity.as_ref()
                )?
            );
        }
        Some("recovery") => {
            let root = Path::new(
                args.get(1)
                    .context("recovery requires evidence directory")?,
            );
            let report = recovery::build(root)?;
            let file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(root.join("acceptance.json"))?;
            serde_json::to_writer_pretty(file, &report)?;
        }
        _ => bail!("unknown evidence command"),
    }
    Ok(())
}

pub fn read(root: &Path, name: &str) -> Result<String> {
    let path = safe_file(root, name)?;
    let text = fs::read_to_string(path)?;
    ensure!(!text.trim().is_empty(), "empty evidence: {name}");
    Ok(text)
}

pub fn json(root: &Path, name: &str) -> Result<Value> {
    serde_json::from_str(&read(root, name)?).with_context(|| format!("invalid evidence: {name}"))
}

fn safe_file(root: &Path, name: &str) -> Result<std::path::PathBuf> {
    let path = Path::new(name);
    ensure!(
        !path.as_os_str().is_empty()
            && path.components().all(|p| matches!(p, Component::Normal(_))),
        "artifact escapes evidence root: {name}"
    );
    let root = root.canonicalize()?;
    let mut current = root;
    for part in path.components() {
        current.push(part);
        ensure!(
            !fs::symlink_metadata(&current)?.file_type().is_symlink(),
            "symlink in evidence: {name}"
        );
    }
    ensure!(current.is_file(), "not regular evidence: {name}");
    Ok(current)
}

pub fn verify_manifest(root: &Path, manifest: &Value, required: &[&str]) -> Result<()> {
    let entries = manifest
        .as_object()
        .context("missing artifact fingerprints")?;
    ensure!(
        !entries.is_empty() && required.iter().all(|key| entries.contains_key(*key)),
        "missing required artifact fingerprints"
    );
    for (name, expected) in entries {
        let bytes = fs::read(safe_file(root, name)?)?;
        ensure!(
            expected.as_str() == Some(&format!("{:x}", Sha256::digest(bytes))),
            "artifact changed: {name}"
        );
    }
    Ok(())
}

pub fn manifest(root: &Path) -> Result<BTreeMap<String, String>> {
    let mut result = BTreeMap::new();
    for entry in walkdir::WalkDir::new(root) {
        let entry = entry?;
        if entry.file_type().is_file() {
            let name = entry
                .path()
                .strip_prefix(root)?
                .to_string_lossy()
                .into_owned();
            if name != "acceptance.json" && name != "outcome.json" {
                result.insert(
                    name.clone(),
                    format!("{:x}", Sha256::digest(fs::read(safe_file(root, &name)?)?)),
                );
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn evidence_requires_exact_bytes_and_cannot_escape_root() -> Result<()> {
        let root = tempfile::tempdir()?;
        fs::write(root.path().join("proof"), "retained")?;
        let expected = json!({"proof": format!("{:x}", Sha256::digest(b"retained"))});
        verify_manifest(root.path(), &expected, &["proof"])?;
        fs::write(root.path().join("proof"), "changed")?;
        assert!(verify_manifest(root.path(), &expected, &["proof"]).is_err());
        assert!(verify_manifest(root.path(), &json!({"../proof":"a"}), &[]).is_err());
        assert!(verify_manifest(root.path(), &json!({}), &[]).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.path().join("proof"), root.path().join("link"))?;
            assert!(read(root.path(), "link").is_err());
        }
        Ok(())
    }
}
