use super::*;
use ayni_orchestrator_domain::EnvironmentChangeAuthorization;
use std::{fs, sync::Arc};

#[test]
fn reviewed_scope_permits_derived_lock_but_preserves_policy_and_other_dependency_files()
-> io::Result<()> {
    let directory = tempfile::tempdir()?;
    let mut spec = super::super::tests::fixture(directory.path())?;
    spec.constraints.environment_change = Some(EnvironmentChangeAuthorization {
        dependency_paths: vec!["Cargo.lock".into()],
    });
    fs::write(directory.path().join(".ayni.lock"), "derived")?;
    fs::write(directory.path().join("Cargo.lock"), "approved")?;
    assert!(super::super::commits::changed_paths(&spec, directory.path()).is_ok());
    fs::write(directory.path().join("package.json"), "unapproved")?;
    assert!(super::super::commits::changed_paths(&spec, directory.path()).is_err());
    fs::remove_file(directory.path().join("package.json"))?;
    fs::write(directory.path().join(".ayni.toml"), "changed")?;
    assert!(super::super::commits::changed_paths(&spec, directory.path()).is_err());
    Ok(())
}

#[tokio::test]
async fn ordinary_tasks_do_not_invoke_lock_regeneration() -> io::Result<()> {
    let directory = tempfile::tempdir()?;
    let context = Context {
        spec: Arc::new(super::super::tests::fixture(directory.path())?),
        workspace: directory.path().to_owned(),
        output: directory.path().join("output"),
        state: Arc::default(),
        serial: Arc::default(),
        activity: super::super::activity::ActivityLog::default(),
    };
    assert!(prepare(&context, Duration::from_secs(1)).await?.is_none());
    Ok(())
}

struct Generator {
    output: Value,
    success: bool,
}
impl LockRunner for Generator {
    async fn generate(&self, context: &Context, base: &str, _: Duration) -> io::Result<bool> {
        assert_eq!(base, "base@sha256:abc");
        fs::write(
            context.workspace.join(".ayni.lock"),
            serde_json::to_vec(&self.output)?,
        )?;
        Ok(self.success)
    }
}

fn reviewed_context(root: &std::path::Path) -> io::Result<(Context, Vec<u8>)> {
    let workspace = root.join("workspace");
    let mut spec = super::super::tests::fixture(&workspace)?;
    let baseline = serde_json::to_vec(
        &json!({"provisioning_base":{"reference":"base","digest":"sha256:abc"},"ayni_version":"0.15.0","mise_version":"2026.6.14","schema_version":"0.9.0","recipe_version":"2","fingerprint":format!("sha256:{}", "a".repeat(64)),"platforms":[{"os":"linux","architecture":"amd64","libc":"glibc"}]}),
    )?;
    fs::write(workspace.join(".ayni.toml"), "policy\n\n")?;
    fs::write(workspace.join(".ayni.lock"), &baseline)?;
    git(&workspace, &["add", "--all"])?;
    git(
        &workspace,
        &[
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.test",
            "commit",
            "-m",
            "chore: freeze baseline",
        ],
    )?;
    spec.execution.revision = git(&workspace, &["rev-parse", "HEAD"])?;
    spec.execution.expected.ayni_version = "0.15.0".into();
    spec.constraints.environment_change = Some(EnvironmentChangeAuthorization {
        dependency_paths: vec!["Cargo.lock".into()],
    });
    fs::write(workspace.join("Cargo.lock"), "reviewed dependencies")?;
    Ok((
        Context {
            spec: Arc::new(spec),
            workspace,
            output: root.join("output"),
            state: Arc::default(),
            serial: Arc::default(),
            activity: super::super::activity::ActivityLog::default(),
        },
        baseline,
    ))
}

#[tokio::test]
async fn regenerated_lock_is_committed_and_evidence_binds_the_final_source() -> io::Result<()> {
    let root = tempfile::tempdir()?;
    let (context, baseline) = reviewed_context(root.path())?;
    let mut output: Value = serde_json::from_slice(&baseline)?;
    output["fingerprint"] = format!("sha256:{}", "b".repeat(64)).into();
    let result = prepare_with(
        &context,
        Duration::from_secs(1),
        &Generator {
            output,
            success: true,
        },
    )
    .await?
    .ok_or_else(|| io::Error::other("result missing"))?;
    assert_eq!(
        result.outcome,
        ayni_orchestrator_runtime::coding::QualityOutcome::Unavailable
    );
    assert!(result.evidence.is_none());
    let commit = super::super::commits::commit_changes(
        &context.spec,
        &context.workspace,
        "chore: retain generated lock",
    )?
    .ok_or_else(|| io::Error::other("commit missing"))?;
    freeze(&context)?;
    validate_retained(&context)?;
    let evidence: Value =
        recovery::read_json(&context.output.join("evidence/environment-change.json"))?;
    assert_eq!(evidence["source_commit"], commit);
    assert_eq!(
        evidence["baseline_lock_digest"],
        ayni_orchestrator_domain::digest::sha256(&baseline)
    );
    assert_eq!(evidence["lock_regeneration"], "succeeded");
    fs::write(context.workspace.join(".ayni.lock"), "tampered")?;
    assert!(validate_retained(&context).is_err());
    Ok(())
}

#[tokio::test]
async fn failed_regeneration_restores_baseline_instead_of_publishing_manual_lock() -> io::Result<()>
{
    let root = tempfile::tempdir()?;
    let (context, baseline) = reviewed_context(root.path())?;
    let result = prepare_with(
        &context,
        Duration::from_secs(1),
        &Generator {
            output: json!({"tampered":true}),
            success: false,
        },
    )
    .await?
    .ok_or_else(|| io::Error::other("result missing"))?;
    assert_eq!(
        result.outcome,
        ayni_orchestrator_runtime::coding::QualityOutcome::Unavailable
    );
    assert_eq!(fs::read(context.workspace.join(".ayni.lock"))?, baseline);
    assert!(
        !super::super::commits::changed_paths(&context.spec, &context.workspace)?
            .contains(&".ayni.lock".into())
    );
    Ok(())
}

#[tokio::test]
async fn successful_generator_with_invalid_output_restores_baseline_and_aborts() -> io::Result<()> {
    let root = tempfile::tempdir()?;
    let (context, baseline) = reviewed_context(root.path())?;
    let error = prepare_with(
        &context,
        Duration::from_secs(1),
        &Generator {
            output: json!({"invalid":true}),
            success: true,
        },
    )
    .await
    .err()
    .ok_or_else(|| io::Error::other("error missing"))?;
    assert!(error.to_string().contains("protected"));
    assert_eq!(fs::read(context.workspace.join(".ayni.lock"))?, baseline);
    Ok(())
}

#[tokio::test]
async fn finalization_cannot_publish_a_lock_without_matching_regeneration_evidence()
-> io::Result<()> {
    let root = tempfile::tempdir()?;
    let (context, baseline) = reviewed_context(root.path())?;
    fs::write(context.workspace.join(".ayni.lock"), "manual")?;
    assert!(freeze(&context).is_err());
    assert!(validate_retained(&context).is_err());
    fs::write(context.workspace.join(".ayni.lock"), &baseline)?;
    let mut output: Value = serde_json::from_slice(&baseline)?;
    output["fingerprint"] = format!("sha256:{}", "b".repeat(64)).into();
    prepare_with(
        &context,
        Duration::from_secs(1),
        &Generator {
            output,
            success: true,
        },
    )
    .await?;
    fs::write(
        context.workspace.join("Cargo.lock"),
        "changed after generation",
    )?;
    assert!(freeze(&context).is_err());
    Ok(())
}

#[test]
fn regeneration_preserves_frozen_provisioning_identity() -> io::Result<()> {
    assert_eq!(
        baseline_base(&json!({"provisioning_base":{"reference":"base","digest":"sha256:abc"}}))?,
        "base@sha256:abc"
    );
    assert!(baseline_base(&json!({})).is_err());
    Ok(())
}
