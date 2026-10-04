use super::*;
use ayni_orchestrator_runtime::assessment::{
    Manifest, Outcome, RunIdentity, Summary, VERSION, sha256,
};
use std::collections::BTreeMap;

fn provider() -> v1::GithubChangeRequest {
    v1::GithubChangeRequest {
        repository_id: "123".into(),
        pull_request_number: 7,
        html_url: "https://github.com/ayni-oss/sandbox/pull/7".into(),
    }
}

fn manifest(revision: &str) -> Manifest {
    Manifest {
        version: VERSION.into(),
        run: RunIdentity {
            repository_id: "123".into(),
            pull_request: 7,
            head: SECOND_REVISION.into(),
            base: REVISION.into(),
            profile_revision: revision.into(),
            workflow_id: 1,
            run_id: 2,
            attempt: 1,
            workflow_sha: REVISION.into(),
            producer_digest: DIGEST.into(),
        },
        runtime_digest: DIGEST.into(),
        lock_fingerprint: DIGEST.into(),
        contract_digest: DIGEST.into(),
        source_before: DIGEST.into(),
        source_after: DIGEST.into(),
        ayni_version: ayni_orchestrator_runtime::ayni_release::CLI_VERSION.into(),
        platform: "linux/amd64".into(),
        exit_code: 0,
        outcome: Outcome::Succeeded,
        artifacts: BTreeMap::new(),
    }
}

fn import(
    store: &mut CleanStore,
    change: &v1::ChangeRequest,
    manifest: &Manifest,
) -> Result<v1::Assessment, Box<dyn std::error::Error>> {
    let artifacts: BTreeMap<String, Vec<u8>> = BTreeMap::from([
        ("signals.json".into(), b"signals".to_vec()),
        ("runtime.json".into(), b"runtime".to_vec()),
    ]);
    let mut manifest = manifest.clone();
    manifest.artifacts = artifacts
        .iter()
        .map(|(name, bytes)| (name.clone(), sha256(bytes)))
        .collect();
    Ok(store.import_review(
        change,
        &manifest,
        &Summary::default(),
        &serde_json::to_vec(&manifest)?,
        &artifacts,
        "https://github.com/ayni-oss/sandbox/actions/runs/2",
    )?)
}

#[test]
fn human_reviews_and_immutable_evidence_survive_restart_and_reject_conflicts()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("state.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    let (repository, profile, _) = ready_profile(&mut store)?;
    let change = store.observe_review(
        &repository.id,
        provider(),
        SECOND_REVISION,
        REVISION,
        v1::ChangeRequestState::Open,
    )?;
    assert!(change.candidate_id.is_empty());
    let mut manifest = manifest(profile.active_revision_id.as_deref().ok_or("revision")?);
    let assessment = import(&mut store, &change, &manifest)?;
    assert_eq!(assessment.id, import(&mut store, &change, &manifest)?.id);
    store.verify_assessment_files(&assessment)?;
    let cursor = store.current_cursor(REVIEW_SERVICE)?;
    assert_eq!(assessment.id, import(&mut store, &change, &manifest)?.id);
    assert_eq!(cursor, store.current_cursor(REVIEW_SERVICE)?);
    manifest.exit_code = 1;
    assert!(import(&mut store, &change, &manifest).is_err());
    drop(store);
    let reopened = CleanStore::open(&path, 3)?;
    reopened.verify_assessment_files(&assessment)?;
    fs::write(
        directory
            .path()
            .join("assessments")
            .join(assessment.id.replace(':', "-"))
            .join("signals.json"),
        b"tampered",
    )?;
    assert!(reopened.verify_assessment_files(&assessment).is_err());
    Ok(())
}

#[test]
fn history_does_not_overwrite_current_provider_head() -> Result<(), Box<dyn std::error::Error>> {
    let mut store = CleanStore::in_memory()?;
    let (repository, profile, _) = ready_profile(&mut store)?;
    let change = store.observe_review(
        &repository.id,
        provider(),
        SECOND_REVISION,
        REVISION,
        v1::ChangeRequestState::Draft,
    )?;
    let mut old = manifest(profile.active_revision_id.as_deref().ok_or("revision")?);
    import(&mut store, &change, &old)?;
    let current = store.observe_review(
        &repository.id,
        provider(),
        REVISION,
        SECOND_REVISION,
        v1::ChangeRequestState::Draft,
    )?;
    old.run.attempt = 2;
    old.outcome = Outcome::Stale;
    import(&mut store, &current, &old)?;
    assert_eq!(store.change_request(&change.id)?.head_revision, REVISION);
    assert_eq!(
        store
            .list_assessments(&change.id, None, None, None)?
            .items
            .len(),
        2
    );
    assert!(
        store
            .list_assessments(&change.id, Some(REVISION), None, None)?
            .items
            .is_empty()
    );
    Ok(())
}

#[test]
fn readiness_rejects_human_prs_even_with_successful_evidence()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let mut store = CleanStore::open(directory.path().join("state.sqlite"), 3)?;
    let (repository, profile, _) = ready_profile(&mut store)?;
    let change = store.observe_review(
        &repository.id,
        provider(),
        SECOND_REVISION,
        REVISION,
        v1::ChangeRequestState::Draft,
    )?;
    let assessment = import(
        &mut store,
        &change,
        &manifest(profile.active_revision_id.as_deref().ok_or("revision")?),
    )?;
    assert!(
        store
            .save_readiness(
                &change.id,
                v1::ReadinessStatus {
                    policy: "READY_AFTER_SUCCESSFUL_ASSESSMENT".into(),
                    state: "pending".into(),
                    source: "orchestrator".into(),
                    authorized_head: SECOND_REVISION.into(),
                    assessment_id: assessment.id,
                    diagnostic: String::new()
                },
                true
            )
            .is_err()
    );
    assert!(store.pending_external_actions("MarkReady")?.is_empty());
    Ok(())
}

#[test]
fn readiness_intent_is_durable_idempotent_and_invalidated_by_new_head()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("state.sqlite");
    let mut store = CleanStore::open(&path, 3)?;
    let (repository, profile, _) = ready_profile(&mut store)?;
    let (_, attempt) = store.create_development_task(
        "task",
        &task_request(&store, &repository.id, &profile.id, "task")?,
    )?;
    let running = store.transition_attempt(
        &attempt.id,
        attempt.resource_version,
        v1::DevelopmentAttemptState::Running,
        None,
    )?;
    store.transition_attempt(
        &attempt.id,
        running.resource_version,
        v1::DevelopmentAttemptState::Succeeded,
        None,
    )?;
    let candidate = store.record_candidate(v1::Candidate {
        development_attempt_id: attempt.id,
        repository_connection_id: repository.id.clone(),
        base_revision: REVISION.into(),
        identity: Some(v1::CandidateIdentity {
            identity: Some(v1::candidate_identity::Identity::Git(v1::GitCandidate {
                commit_sha: SECOND_REVISION.into(),
            })),
        }),
        state: v1::CandidateState::Available as i32,
        changed_paths: vec!["src/lib.rs".into()],
        ..Default::default()
    })?;
    let request = v1::PublishCandidateRequest {
        idempotency_key: "publish".into(),
        candidate_id: candidate.id,
        title: "feat: test".into(),
        body: String::new(),
        base_branch: "main".into(),
        draft: true,
    };
    let change = store.publish_candidate("publish", &request)?;
    let change = store.reconcile_change_request(
        "publish",
        change.resource_version,
        v1::ChangeRequestProvider {
            provider: Some(v1::change_request_provider::Provider::Github(provider())),
        },
        v1::ChangeRequestState::Draft,
        SECOND_REVISION,
        None,
    )?;
    let assessment = import(
        &mut store,
        &change,
        &manifest(profile.active_revision_id.as_deref().ok_or("revision")?),
    )?;
    let mut status = v1::ReadinessStatus {
        policy: "DRAFT_ONLY".into(),
        state: "pending".into(),
        source: "orchestrator".into(),
        authorized_head: SECOND_REVISION.into(),
        assessment_id: assessment.id.clone(),
        diagnostic: "awaiting_provider_transition".into(),
    };
    assert!(
        store
            .save_readiness(&change.id, status.clone(), true)
            .is_err()
    );
    status.policy = "READY_AFTER_SUCCESSFUL_ASSESSMENT".into();
    store.save_readiness(&change.id, status.clone(), true)?;
    store.save_readiness(&change.id, status.clone(), true)?;
    assert_eq!(store.pending_external_actions("MarkReady")?.len(), 1);
    drop(store);
    let mut store = CleanStore::open(path, 3)?;
    assert_eq!(store.pending_external_actions("MarkReady")?.len(), 1);
    assert_eq!(store.change_request(&change.id)?.readiness, Some(status));
    let current = store.observe_review(
        &repository.id,
        provider(),
        REVISION,
        REVISION,
        v1::ChangeRequestState::Draft,
    )?;
    assert_eq!(
        current.readiness.as_ref().map(|r| r.state.as_str()),
        Some("obsolete")
    );
    assert!(store.pending_external_actions("MarkReady")?.is_empty());
    assert_eq!(
        store.assessment(&assessment.id)?.head_revision,
        SECOND_REVISION
    );
    Ok(())
}
