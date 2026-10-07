use super::*;
use crate::operations::runtime::git;
use std::{fs, sync::Arc};

type TestResult = Result<(), Box<dyn std::error::Error>>;
fn journal() -> Journal {
    Journal {
        task_id: "task-test".into(),
        attempt_id: "attempt-test".into(),
        repository: "team/repository".into(),
        branch: "task/attempt-test".into(),
        base_branch: "main".into(),
        head_commit: "a".repeat(40),
        receipt: None,
    }
}
fn pull(journal: &Journal) -> Value {
    json!({"number": 42, "html_url": "https://github.com/team/repository/pull/42", "state": "open", "draft": true, "body": marker(journal), "head": {"repo": {"full_name": journal.repository}, "ref": journal.branch, "sha": journal.head_commit}, "base": {"repo": {"full_name": journal.repository}, "ref": journal.base_branch}})
}
fn context(root: &Path) -> io::Result<Context> {
    let workspace = root.join("workspace");
    let spec = super::super::tests::fixture(&workspace)?;
    Ok(Context {
        spec: Arc::new(spec),
        workspace,
        output: root.join("output"),
        state: Arc::new(tokio::sync::Mutex::new(
            super::super::sessions::SessionState::default(),
        )),
        serial: Arc::new(tokio::sync::Mutex::new(())),
        activity: super::super::activity::ActivityLog::default(),
    })
}

#[test]
fn markdown_content_cannot_inject_html_markers_or_links() {
    assert_eq!(
        escape("<script>[link](url)*`"),
        "&lt;script&gt;\\[link\\](url)\\*\\`"
    );
}

#[test]
fn matching_open_draft_is_adopted_with_exact_frozen_identity() -> TestResult {
    let journal = journal();
    let accepted = receipt(&journal, &pull(&journal))?;
    assert_eq!(accepted.repository, journal.repository);
    assert_eq!(accepted.branch, journal.branch);
    assert_eq!(accepted.base_branch, journal.base_branch);
    assert_eq!(accepted.head_commit, journal.head_commit);
    assert_eq!(accepted.number, 42);
    assert!(accepted.draft);
    Ok(())
}

#[test]
fn conflicting_or_malformed_provider_responses_cannot_be_adopted() -> TestResult {
    let journal = journal();
    for (pointer, replacement) in [
        ("/head/sha", json!("b".repeat(40))),
        ("/head/ref", json!("human-branch")),
        ("/base/ref", json!("release")),
        ("/head/repo/full_name", json!("fork/repository")),
        ("/base/repo/full_name", json!("team/another")),
        ("/body", json!("Unrelated human PR")),
        ("/draft", json!(false)),
        ("/state", json!("closed")),
        ("/head/sha", json!(null)),
        ("/number", json!(0)),
        ("/number", json!("42")),
        ("/html_url", json!("https://example.com/pull/42")),
    ] {
        let mut value = pull(&journal);
        *value.pointer_mut(pointer).ok_or("fixture path missing")? = replacement;
        assert!(
            receipt(&journal, &value).is_err(),
            "accepted conflicting field {pointer}"
        );
    }
    assert!(receipt(&journal, &json!({})).is_err());
    Ok(())
}

#[tokio::test]
async fn persisted_delivery_receipt_is_reused_without_provider_calls_or_credentials() -> TestResult
{
    let directory = tempfile::tempdir()?;
    let context = context(directory.path())?;
    let mut result = super::super::empty_result();
    result.commit = Some(git(&context.workspace, &["rev-parse", "HEAD"])?);
    let mut journal = intent(&context, &result)?;
    journal.receipt = Some(receipt(&journal, &pull(&journal))?);
    recovery::write_json(&context.output.join("publication.json"), &journal)?;
    deliver(&context, &mut result).await?;
    assert_eq!(result.publication, PublicationOutcome::Published);
    assert_eq!(result.pull_request, journal.receipt);
    let mut restarted = super::super::empty_result();
    restarted.commit.clone_from(&result.commit);
    deliver(&context, &mut restarted).await?;
    assert_eq!(restarted.pull_request, result.pull_request);
    Ok(())
}

#[tokio::test]
async fn corrupted_persisted_receipt_is_rejected_before_any_delivery() -> TestResult {
    let directory = tempfile::tempdir()?;
    let context = context(directory.path())?;
    let mut result = super::super::empty_result();
    result.commit = Some(git(&context.workspace, &["rev-parse", "HEAD"])?);
    let mut journal = intent(&context, &result)?;
    let mut accepted = receipt(&journal, &pull(&journal))?;
    accepted.head_commit = "b".repeat(40);
    journal.receipt = Some(accepted);
    recovery::write_json(&context.output.join("publication.json"), &journal)?;
    assert!(deliver(&context, &mut result).await.is_err());
    assert!(result.pull_request.is_none());
    assert_ne!(result.publication, PublicationOutcome::Published);
    Ok(())
}

#[cfg(unix)]
#[test]
fn projected_credentials_follow_atomic_rotations_inside_the_trusted_mount() -> TestResult {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    fs::create_dir(root.join("generation-one"))?;
    fs::create_dir(root.join("generation-two"))?;
    fs::write(root.join("generation-one/token"), b"fake-attempt-one")?;
    fs::write(root.join("generation-two/token"), b"fake-attempt-two")?;
    symlink("generation-one", root.join("..data"))?;
    symlink("..data/token", root.join("token"))?;
    assert_eq!(projected_credential(root, "token")?, b"fake-attempt-one");
    symlink("generation-two", root.join("..data-new"))?;
    fs::rename(root.join("..data-new"), root.join("..data"))?;
    assert_eq!(projected_credential(root, "token")?, b"fake-attempt-two");
    Ok(())
}

#[cfg(unix)]
#[test]
fn credential_projections_cannot_escape_mount_or_exceed_input_bound() -> TestResult {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("mount");
    fs::create_dir(&root)?;
    let outside = directory.path().join("outside-fake-token");
    fs::write(&outside, b"fake-outside-credential")?;
    symlink(&outside, root.join("token"))?;
    assert!(projected_credential(&root, "token").is_err());
    assert!(projected_credential(&root, "../outside-fake-token").is_err());
    fs::remove_file(root.join("token"))?;
    fs::write(root.join("token"), vec![b'x'; 16_385])?;
    assert!(projected_credential(&root, "token").is_err());
    Ok(())
}

#[test]
fn credential_bearing_push_discards_source_git_configuration_idempotently() -> TestResult {
    let root = tempfile::tempdir()?;
    let context = context(root.path())?;
    git(
        &context.workspace,
        &["config", "credential.helper", "source-helper"],
    )?;
    git(
        &context.workspace,
        &[
            "config",
            "url.https://example.invalid/.insteadOf",
            "https://github.com/",
        ],
    )?;
    reset_git_configuration(&context.workspace)?;
    reset_git_configuration(&context.workspace)?;
    let config = fs::read_to_string(context.workspace.join(".git/config"))?;
    assert!(!config.contains("helper"));
    assert!(!config.contains("insteadOf"));
    assert!(config.contains("hooksPath = /dev/null"));
    assert_eq!(
        git(&context.workspace, &["rev-parse", "HEAD"])?,
        context.spec.execution.revision
    );
    Ok(())
}
