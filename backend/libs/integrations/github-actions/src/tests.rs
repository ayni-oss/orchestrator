use super::*;
use serde_json::json;

fn fixture() -> (Manifest, BTreeMap<String, Vec<u8>>) {
    let hash = format!("sha256:{}", "a".repeat(64));
    // Structural fixture only: cryptographic verification belongs to Ayni and
    // workflow authority must be independently supplied by the importer.
    let metadata = json!({"key_id":"approved", "signature":"b".repeat(128), "certificate": {
        "schema_version":"2", "lock_fingerprint":hash,"platform":"linux/amd64","ayni_version":ayni_orchestrator_profile_ayni::ayni_release::CLI_VERSION,
        "tool_inventory_digest":hash,"protected_content_root":hash
    }});
    let bytes = metadata.to_string().into_bytes();
    let signals = json!({
        "schema_version":ayni_orchestrator_profile_ayni::ayni_release::SIGNALS_SCHEMA_VERSION, "execution_mode":"managed", "environment_lock_fingerprint":hash,
        "ayni_version":ayni_orchestrator_profile_ayni::ayni_release::CLI_VERSION, "contract_digest":hash,"invocation":{"command":"check"},"tool_versions":[{"tool":"rust","version":"1.94.0"}],
        "prebuilt_runtime":{"metadata_path":"/etc/ayni/runtime.json","metadata_digest":sha256(&bytes),"certificate":metadata},
        "completion":{"scope":"repository","state":"complete","expected_targets":1,"detected_targets":1,"completed_targets":1,"skipped_targets":0,"issues":[]},
        "aggregate":{"total_rows":1,"passing_rows":1,"failing_rows":0,"warning_offenders":0,"failing_offenders":0},
        "rows":[{"kind":"test","language":"rust","scope":{},"pass":true}]
    });
    let artifacts: BTreeMap<String, Vec<u8>> = BTreeMap::from([
        ("runtime.json".into(), bytes),
        ("signals.json".into(), signals.to_string().into_bytes()),
    ]);
    let manifest = Manifest {
        version: VERSION.into(),
        run: RunIdentity {
            repository_id: "42".into(),
            pull_request: 7,
            head: "a".repeat(40),
            base: "b".repeat(40),
            profile_revision: "revision:1".into(),
            workflow_id: 12,
            run_id: 13,
            attempt: 1,
            workflow_sha: "c".repeat(40),
            producer_digest: hash.clone(),
        },
        runtime_digest: hash.clone(),
        lock_fingerprint: hash.clone(),
        contract_digest: hash.clone(),
        source_before: hash.clone(),
        source_after: hash,
        ayni_version: ayni_orchestrator_profile_ayni::ayni_release::CLI_VERSION.into(),
        platform: "linux/amd64".into(),
        exit_code: 0,
        outcome: Outcome::Succeeded,
        artifacts: artifacts
            .iter()
            .map(|(k, v)| (k.clone(), sha256(v)))
            .collect(),
    };
    (manifest, artifacts)
}

fn admit(
    manifest: &Manifest,
    artifacts: &BTreeMap<String, Vec<u8>>,
) -> Result<(Manifest, Summary), &'static str> {
    let (trusted, _) = fixture();
    let bytes = serde_json::to_vec(manifest).map_err(|_| "fixture_invalid")?;
    validate(
        &bytes,
        artifacts,
        &Admission {
            run: &trusted.run,
            runtime_digest: &trusted.runtime_digest,
            lock_fingerprint: &trusted.lock_fingerprint,
            contract_digest: &trusted.contract_digest,
            current_head: &trusted.run.head,
            platform: &trusted.platform,
            ayni_version: &trusted.ayni_version,
        },
    )
}

fn mutate_signals(
    manifest: &mut Manifest,
    artifacts: &mut BTreeMap<String, Vec<u8>>,
    change: impl FnOnce(&mut Value),
) -> Result<(), Box<dyn std::error::Error>> {
    let mut signals: Value = serde_json::from_slice(&artifacts["signals.json"])?;
    change(&mut signals);
    let bytes = serde_json::to_vec(&signals)?;
    manifest
        .artifacts
        .insert("signals.json".into(), sha256(&bytes));
    artifacts.insert("signals.json".into(), bytes);
    Ok(())
}

#[test]
fn imports_complete_success_and_distinct_quality_failure() -> Result<(), Box<dyn std::error::Error>>
{
    let (mut manifest, mut artifacts) = fixture();
    assert_eq!(admit(&manifest, &artifacts)?.1.completed, 1);
    mutate_signals(&mut manifest, &mut artifacts, |s| {
        s["rows"][0]["pass"] = json!(false);
        s["aggregate"]["passing_rows"] = json!(0);
        s["aggregate"]["failing_rows"] = json!(1);
    })?;
    assert!(admit(&manifest, &artifacts).is_err());
    manifest.outcome = Outcome::QualityFailed;
    manifest.exit_code = 1;
    assert_eq!(
        admit(&manifest, &artifacts)?.0.outcome,
        Outcome::QualityFailed
    );
    Ok(())
}

#[test]
fn rejects_forged_producer_source_and_incorrect_runtime_bindings() {
    let (trusted, artifacts) = fixture();
    for mutate in [
        |m: &mut Manifest| m.run.run_id += 1,
        |m: &mut Manifest| m.run.attempt += 1,
        |m: &mut Manifest| m.run.repository_id = "43".into(),
        |m: &mut Manifest| m.run.pull_request += 1,
        |m: &mut Manifest| m.run.workflow_id += 1,
        |m: &mut Manifest| m.run.workflow_sha = "f".repeat(40),
        |m: &mut Manifest| m.run.producer_digest = format!("sha256:{}", "f".repeat(64)),
        |m: &mut Manifest| m.source_after = format!("sha256:{}", "f".repeat(64)),
        |m: &mut Manifest| m.runtime_digest = format!("sha256:{}", "f".repeat(64)),
        |m: &mut Manifest| m.contract_digest = format!("sha256:{}", "f".repeat(64)),
        |m: &mut Manifest| m.exit_code = 137,
    ] {
        let mut manifest = trusted.clone();
        mutate(&mut manifest);
        assert!(admit(&manifest, &artifacts).is_err());
    }
}

#[test]
fn rejects_self_consistent_but_unsigned_host_and_incomplete_accounting()
-> Result<(), Box<dyn std::error::Error>> {
    for mutate in [
        |v: &mut Value| v["execution_mode"] = json!("host"),
        |v: &mut Value| v["prebuilt_runtime"]["certificate"]["key_id"] = json!("unsigned"),
        |v: &mut Value| v["prebuilt_runtime"]["metadata_digest"] = json!("fake"),
        |v: &mut Value| v["completion"]["completed_targets"] = json!(0),
        |v: &mut Value| v["completion"]["scope"] = json!("selected"),
        |v: &mut Value| v["aggregate"]["total_rows"] = json!(3),
        |v: &mut Value| v["rows"][0]["pass"] = json!("true"),
    ] {
        let (mut manifest, mut artifacts) = fixture();
        mutate_signals(&mut manifest, &mut artifacts, mutate)?;
        assert!(admit(&manifest, &artifacts).is_err());
    }
    Ok(())
}

#[test]
fn rejects_tampering_missing_and_oversized_evidence() {
    let (manifest, mut artifacts) = fixture();
    artifacts
        .get_mut("signals.json")
        .into_iter()
        .for_each(|bytes| bytes.push(b' '));
    assert!(admit(&manifest, &artifacts).is_err());
    artifacts.remove("signals.json");
    assert!(admit(&manifest, &artifacts).is_err());
    artifacts.insert("signals.json".into(), vec![b' '; MAX_BUNDLE_BYTES + 1]);
    assert!(admit(&manifest, &artifacts).is_err());
}

#[test]
fn preserves_stale_evidence_without_authorizing_current_success()
-> Result<(), Box<dyn std::error::Error>> {
    let (manifest, artifacts) = fixture();
    let (imported, _) = validate(
        &serde_json::to_vec(&manifest)?,
        &artifacts,
        &Admission {
            run: &manifest.run,
            runtime_digest: &manifest.runtime_digest,
            lock_fingerprint: &manifest.lock_fingerprint,
            contract_digest: &manifest.contract_digest,
            current_head: &"f".repeat(40),
            platform: &manifest.platform,
            ayni_version: &manifest.ayni_version,
        },
    )?;
    assert_eq!(imported.outcome, Outcome::Stale);
    Ok(())
}

#[test]
fn unsupported_cancelled_and_infrastructure_outcomes_cannot_become_success()
-> Result<(), Box<dyn std::error::Error>> {
    let (mut manifest, _) = fixture();
    manifest.artifacts.clear();
    for outcome in [
        Outcome::Unsupported,
        Outcome::Cancelled,
        Outcome::PolicyRejected,
        Outcome::InfrastructureFailed,
    ] {
        manifest.outcome = outcome;
        assert_eq!(admit(&manifest, &BTreeMap::new())?.0.outcome, outcome);
    }
    manifest.outcome = Outcome::Succeeded;
    assert!(admit(&manifest, &BTreeMap::new()).is_err());
    Ok(())
}
