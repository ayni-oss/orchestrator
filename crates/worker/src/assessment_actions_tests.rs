use super::*;

#[test]
fn launcher_preserves_entrypoint_and_excludes_host_authority() {
    let paths = vec!["src/nested/lib.rs".into(), "Cargo.lock".into()];
    let args = launch_arguments(
        Path::new("/tmp/source"),
        &paths,
        "registry/image@sha256:digest".into(),
        7,
    );
    for required in [
        "--read-only",
        "--network=none",
        "--cap-drop=ALL",
        "--security-opt=no-new-privileges",
        "--user=10002:10001",
        "--pids-limit=2048",
    ] {
        assert!(args.iter().any(|a| a == required));
    }
    let joined = args.join(" ");
    assert!(joined.contains("source=/tmp/source/src,target=/workspace/src"));
    assert!(joined.contains("source=/tmp/source/src/nested,target=/workspace/src/nested"));
    assert!(joined.contains("target=/workspace/src/nested/lib.rs,readonly"));
    assert!(joined.contains("target=/workspace/.git,readonly"));
    for forbidden in [
        "--entrypoint",
        "docker.sock",
        "GH_TOKEN",
        "SIGNING_KEY",
        "--privileged",
        "--network=host",
        "--pid=host",
    ] {
        assert!(!joined.contains(forbidden));
    }
}

#[test]
fn comment_replacement_preserves_human_text_and_is_idempotent()
-> Result<(), Box<dyn std::error::Error>> {
    let old = "Human preface\n<!-- orchestrator-assessment:v1 -->\nold\n<!-- /orchestrator-assessment:v1 -->\nHuman follow-up";
    let block = "<!-- orchestrator-assessment:v1 -->\nnew\n<!-- /orchestrator-assessment:v1 -->";
    let updated = replace_summary_block(old, block)?;
    assert!(updated.starts_with("Human preface\n"));
    assert!(updated.ends_with("\nHuman follow-up"));
    assert!(!updated.contains("\nold\n"));
    assert_eq!(replace_summary_block(&updated, block)?, updated);
    assert!(replace_summary_block("human comment", block).is_err());
    assert!(replace_summary_block(&format!("{old}{old}"), block).is_err());
    Ok(())
}

#[test]
fn source_measurement_detects_changes_and_rejects_symlinks()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    fs::write(directory.path().join("source.rs"), b"original")?;
    let paths = vec!["source.rs".into()];
    let before = source_digest(directory.path(), &paths)?;
    fs::write(directory.path().join("source.rs"), b"changed")?;
    assert_ne!(before, source_digest(directory.path(), &paths)?);
    #[cfg(unix)]
    {
        fs::remove_file(directory.path().join("source.rs"))?;
        std::os::unix::fs::symlink("/etc/passwd", directory.path().join("source.rs"))?;
        assert!(source_digest(directory.path(), &paths).is_err());
    }
    Ok(())
}
