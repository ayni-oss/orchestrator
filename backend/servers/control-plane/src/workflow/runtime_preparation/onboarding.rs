//! Restartable, one-shot default-branch onboarding using canonical inspection and preparation.
use super::{RequestContext, WorkflowGrpc, operations, profile::RuntimeConfiguration, v1};
use ayni_orchestrator_workflow_postgres::store::repository_runtime::{Claim, Run, Settings};
use tokio::sync::watch;
use tonic::Request;
use v1::repository_service_server::RepositoryService as _;

pub(super) fn settings(value: v1::StartRepositoryRuntimeRequest) -> Settings {
    Settings {
        repository_id: value.repository_id,
        expected_connection_revision: value.expected_connection_revision,
        registry_connection_id: value.registry_connection_id,
        signer_id: value.signer_id,
        platform: value.platform,
        max_seconds: value.max_seconds,
        max_builds: value.max_builds,
        allow_build: value.allow_build,
        allow_replace: value.allow_replace,
        branch: value.branch,
        expected_commit_sha: value.expected_commit_sha,
        expected_registry_revision: value.expected_registry_revision,
        expected_signer_revision: value.expected_signer_revision,
    }
}
pub(super) fn public(run: Run) -> v1::RepositoryRuntimeRun {
    let s = run.settings;
    v1::RepositoryRuntimeRun {
        id: run.id,
        idempotency_key: run.idempotency_key,
        settings: Some(v1::StartRepositoryRuntimeRequest {
            idempotency_key: String::new(),
            repository_id: s.repository_id,
            expected_connection_revision: s.expected_connection_revision,
            registry_connection_id: s.registry_connection_id,
            signer_id: s.signer_id,
            platform: s.platform,
            max_seconds: s.max_seconds,
            max_builds: s.max_builds,
            allow_build: s.allow_build,
            allow_replace: s.allow_replace,
            branch: s.branch,
            expected_commit_sha: s.expected_commit_sha,
            expected_registry_revision: s.expected_registry_revision,
            expected_signer_revision: s.expected_signer_revision,
        }),
        state: run.state,
        safe_reason: run.safe_reason,
        commit_sha: run.commit_sha,
        inspection_id: run.inspection_id,
        preparation_id: run.preparation_id,
    }
}
pub(in crate::workflow) async fn run(service: WorkflowGrpc, mut shutdown: watch::Receiver<bool>) {
    let mut ticks = tokio::time::interval(std::time::Duration::from_secs(2));
    loop {
        tokio::select! {
            _=shutdown.changed()=>break,
            _=ticks.tick()=>{
                if *shutdown.borrow(){break;}
                match service.store.claim_repository_runtime().await {
                    Ok(Some(claim))=>{
                        // Lease outlives the bounded provider step. Dropped/restarted workers
                        // recover deterministic commands rather than a second build allowance.
                        let outcome=tokio::time::timeout(std::time::Duration::from_secs(45),step(&service,&claim)).await.unwrap_or(Err("step_unavailable"));
                        if let Err(reason)=outcome {
                            if reason == "step_unavailable" { continue; }
                            if let Err(error)=service.store.advance_repository_runtime(&claim,"blocked",reason).await {
                                tracing::warn!(run_id=%claim.run.id,%error,"Repository runtime step will recover after lease expiry");
                            }
                        }
                    }
                    Ok(None)=>{},
                    Err(error)=>tracing::warn!(%error,"Repository runtime queue unavailable"),
                }
            }
        }
    }
}
async fn step(service: &WorkflowGrpc, claim: &Claim) -> Result<(), &'static str> {
    let r = &claim.run;
    let s = &r.settings;
    if r.state != "admitting" {
        service
            .store
            .validate_repository_runtime_configuration(r)
            .await
            .map_err(|_| "configuration_changed")?;
    }
    let (state, value) = match r.state.as_str() {
        "resolving" => {
            let result = service
                .resolve_repository_default_commit(Request::new(
                    v1::ResolveRepositoryDefaultCommitRequest {
                        repository_id: s.repository_id.clone(),
                        expected_connection_revision: s.expected_connection_revision,
                        branch: s.branch.clone(),
                    },
                ))
                .await
                .map_err(|_| "source_unavailable")?
                .into_inner();
            if !result.safe_reason.is_empty() {
                return Err("source_unavailable");
            }
            if !s.expected_commit_sha.is_empty() && result.commit_sha != s.expected_commit_sha {
                return Err("branch_head_changed");
            }
            ("inspecting", result.commit_sha)
        }
        "inspecting" => {
            let result = service
                .inspect_repository(Request::new(v1::InspectRepositoryRequest {
                    idempotency_key: r.inspection_key.clone(),
                    repository_id: s.repository_id.clone(),
                    expected_connection_revision: s.expected_connection_revision,
                    commit_sha: r.commit_sha.clone(),
                }))
                .await
                .map_err(|_| "inspection_failed")?
                .into_inner()
                .inspection
                .ok_or("inspection_failed")?;
            if result.state != "completed" {
                return Err("inspection_failed");
            }
            ("admitting", result.id)
        }
        "admitting" => ("completed", admit(service, r).await?),
        _ => return Ok(()),
    };
    service
        .store
        .advance_repository_runtime(claim, state, &value)
        .await
        .map_err(|_| "step_unavailable")
}

async fn admit(service: &WorkflowGrpc, r: &Run) -> Result<String, &'static str> {
    let s = &r.settings;
    let context = RequestContext::new(&Request::new(()));
    let request = v1::StartRuntimePreparationRequest {
        idempotency_key: r.preparation_key.clone(),
        repository_id: s.repository_id.clone(),
        inspection_id: r.inspection_id.clone(),
        registry_connection_id: s.registry_connection_id.clone(),
        signer_id: s.signer_id.clone(),
        platform: s.platform.clone(),
        max_seconds: s.max_seconds,
        max_builds: s.max_builds,
        allow_build: s.allow_build,
        allow_replace: s.allow_replace,
    };
    // Recover accepted work even if the deployment profile was disabled afterward.
    let command = ayni_orchestrator_workflow_postgres::store::runtime_preparations::Start {
        key: &r.preparation_key,
        repository_id: &s.repository_id,
        inspection_id: &r.inspection_id,
        registry_connection_id: &s.registry_connection_id,
        signer_id: &s.signer_id,
        platform: &s.platform,
        max_seconds: s.max_seconds,
        max_builds: s.max_builds,
        allow_build: s.allow_build,
        allow_replace: s.allow_replace,
    };
    let existing = service
        .store
        .replay_runtime_start(&command)
        .await
        .map_err(|_| "preparation_admission_failed")?;
    let operation = if let Some(operation) = existing {
        operation
    } else {
        service
            .store
            .validate_repository_runtime_configuration(r)
            .await
            .map_err(|_| "configuration_changed")?;
        let inspected = service
            .store
            .get_runtime_inspection_by_id(&r.inspection_id)
            .await
            .map_err(|_| "inspection_failed")?;
        if inspected.result.as_ref().is_none_or(|v| {
            v.findings.iter().any(|f| {
                !matches!(
                    f.code.as_str(),
                    "adapter_discovery_pending" | "runtime_verification_pending"
                )
            })
        }) {
            return Err("inspection_findings");
        }
        RuntimeConfiguration::from_environment().map_err(|_| "runtime_profile_unavailable")?;
        operations::start_reviewed(
            service,
            &context,
            &request,
            Some((r.registry_revision, r.signer_revision)),
        )
        .await
        .map_err(|status| {
            if matches!(
                status.code(),
                tonic::Code::Unavailable | tonic::Code::Internal | tonic::Code::DeadlineExceeded
            ) {
                "step_unavailable"
            } else {
                "preparation_admission_failed"
            }
        })?
    };
    Ok(operation.id)
}
