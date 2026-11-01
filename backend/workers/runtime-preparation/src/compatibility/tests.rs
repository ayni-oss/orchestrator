use super::*;
use ayni_core::{CompletionScope, ExecutionMode};
use std::fs;
type TestResult = Result<(), Box<dyn std::error::Error>>;
type Fixture = (RunArtifact, Expected, Inputs, Vec<TargetIdentity>);
fn fixture() -> Result<Fixture, Box<dyn std::error::Error>> {
    use ayni_core::{
        ArtifactToolVersion, Budget, EnvironmentCertificate, InvocationContext, Offenders,
        OutputContext, PrebuiltRuntimeIdentity, RunArtifactMetadata, RunCompletion, Scope,
        SignalKind, SignalResult, SignalRow, TestBudget,
    };
    let lock: EnvironmentLock = serde_json::from_str(include_str!(
        "../../../../../dev/fixtures/runtime-preparation/source/.ayni.lock"
    ))?;
    let targets: Vec<_> = lock
        .targets()
        .iter()
        .map(|value| value.target.clone())
        .collect();
    let digest = sha256_fingerprint(b"synthetic fixture identity");
    let certificate = EnvironmentCertificate::new(
        lock.fingerprint(),
        native_platform(),
        lock.ayni_version(),
        &digest,
        &digest,
    )?;
    let signing = ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
    let certificate = EnvironmentCertificateEnvelope::sign(certificate, "fixture", &signing)?;
    let expected = Expected {
        executable: digest.clone(),
        certificate: sha256_fingerprint(serde_json::to_vec(&certificate)?),
        lock: lock.fingerprint().into(),
        source: digest.clone(),
        platform: native_platform().into(),
        seconds: 30,
    };
    let scope = Scope {
        workspace_root: WORKSPACE.into(),
        ..Scope::default()
    };
    let metadata = RunArtifactMetadata {
        generated_at: "2026-10-31T00:00:00Z".into(),
        ayni_version: lock.ayni_version().into(),
        invocation: InvocationContext {
            command: "check".into(),
            languages: targets.iter().map(|target| target.language).collect(),
            scope: Some(scope.clone()),
        },
        output: OutputContext {
            format: "json".into(),
            destination: "stdout".into(),
        },
        config_path: "/workspace/.ayni.toml".into(),
        repository_root: WORKSPACE.into(),
        execution_mode: ExecutionMode::Managed,
        contract_digest: digest.clone(),
        environment_lock_fingerprint: Some(expected.lock.clone()),
        prebuilt_runtime: Some(PrebuiltRuntimeIdentity {
            metadata_path: "/etc/ayni/runtime.json".into(),
            metadata_digest: expected.certificate.clone(),
            certificate: certificate.clone(),
        }),
        source_fingerprint: digest.clone(),
        tool_versions: vec![ArtifactToolVersion {
            tool: "node".into(),
            version: "22.14.0".into(),
        }],
    };
    // A legitimate test-metric failure is separate from unavailable tooling.
    let rows = vec![SignalRow {
        kind: SignalKind::Test,
        language: targets[0].language,
        scope,
        pass: false,
        result: SignalResult::Test(ayni_core::TestResult {
            total_tests: 1,
            passed: 0,
            failed: 1,
            duration_ms: None,
            runner: "cargo".into(),
            failure: None,
        }),
        budget: Budget::Test(TestBudget {}),
        offenders: Offenders::Test(vec![]),
    }];
    let artifact = RunArtifact::new(
        metadata,
        RunCompletion::complete(CompletionScope::Repository, targets.len() as u64),
        rows,
    )?;
    let bytes = serde_json::to_vec(&artifact)?;
    let artifact: RunArtifact = serde_json::from_slice(&bytes)?;
    Ok((
        artifact,
        expected,
        Inputs {
            lock,
            certificate,
            policy_digest: digest,
        },
        targets,
    ))
}
#[test]
fn managed_compatibility_accepts_quality_metrics_failure_but_rejects_incomplete_tooling()
-> TestResult {
    let (artifact, expected, inputs, targets) = fixture()?;
    assert!(!artifact.rows[0].pass);
    assert!(validate_artifact(&artifact, &expected, &inputs, &targets).is_ok());
    let mut host = artifact.clone();
    host.metadata.execution_mode = ExecutionMode::Host;
    assert!(validate_artifact(&host, &expected, &inputs, &targets).is_err());
    let mut incomplete = artifact.clone();
    incomplete.completion.completed_targets = 0;
    assert!(validate_artifact(&incomplete, &expected, &inputs, &targets).is_err());
    let mut tools = artifact.clone();
    tools.metadata.tool_versions.clear();
    assert!(validate_artifact(&tools, &expected, &inputs, &targets).is_err());
    let mut scoped = artifact.clone();
    scoped
        .metadata
        .invocation
        .scope
        .as_mut()
        .ok_or("scope")?
        .file = Some("src/lib.rs".into());
    assert!(validate_artifact(&scoped, &expected, &inputs, &targets).is_err());
    let mut failed = artifact.clone();
    if let ayni_core::SignalResult::Test(test) = &mut failed.rows[0].result {
        test.failure = Some(ayni_core::CommandFailure {
            category: "tool".into(),
            classification: "missing".into(),
            command: "cargo".into(),
            cwd: WORKSPACE.into(),
            exit_code: Some(127),
            message: "fixture".into(),
        });
    }
    assert!(validate_artifact(&failed, &expected, &inputs, &targets).is_err());
    let mut unsupported = serde_json::to_value(&artifact)?;
    unsupported["schema_version"] = "99.0.0".into();
    assert!(serde_json::from_value::<RunArtifact>(unsupported).is_err());
    Ok(())
}
#[cfg(unix)]
#[test]
fn prepared_tree_binds_modes_and_dependencies_while_artifacts_remain_separate() -> TestResult {
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    let source = tempfile::tempdir()?;
    let prepared = tempfile::tempdir()?;
    fs::write(source.path().join("file"), b"source")?;
    fs::write(prepared.path().join("file"), b"source")?;
    assert!(compare_originals(source.path(), prepared.path()).is_ok());
    let original = prepared_digest(prepared.path())?;
    fs::create_dir(prepared.path().join(".ayni"))?;
    fs::write(
        prepared.path().join(".ayni/signals.json"),
        b"untrusted artifacts",
    )?;
    assert_eq!(original, prepared_digest(prepared.path())?);
    fs::write(prepared.path().join("dependency"), b"prepared")?;
    assert_ne!(original, prepared_digest(prepared.path())?);
    assert!(compare_originals(source.path(), prepared.path()).is_ok());
    fs::set_permissions(
        prepared.path().join("file"),
        fs::Permissions::from_mode(0o755),
    )?;
    assert!(compare_originals(source.path(), prepared.path()).is_err());
    fs::remove_file(prepared.path().join("file"))?;
    symlink(source.path().join("file"), prepared.path().join("file"))?;
    assert!(compare_originals(source.path(), prepared.path()).is_err());
    Ok(())
}
