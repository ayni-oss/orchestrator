use super::admission::current;
use super::*;
use ayni_orchestrator_events::execution_diagnostics::{
    CERTIFICATION_DIAGNOSTIC, CertificationDiagnostic, Diagnostic, Kind,
};
use ayni_orchestrator_events::runtime_preparation::{
    BUILD_REPORT_KEY, BUILD_STEPS, BuildMilestone, BuildReport,
};
pub(super) mod transitions;

async fn active_attempt(
    tx: &Transaction<'_>,
    op: &Operation,
) -> Result<Option<StageAttempt>, StoreError> {
    tx.query_opt("SELECT * FROM workflow_runtime_preparation_attempts WHERE operation_id=$1 AND generation=$2 FOR UPDATE",&[&op.id,&integer(op.generation)?]).await?.as_ref().map(attempt).transpose()
}
fn same_binding(
    op: &Operation,
    a: &StageAttempt,
    b: &Binding,
    require_uid: bool,
) -> Result<(), StoreError> {
    let operation_matches = op.id == b.operation_id;
    if !operation_matches
        || op.generation != b.generation
        || op.stage != b.stage
        || a.command_digest != b.command_digest
        || require_uid && (b.pod_uid.is_empty() || a.pod_uid != b.pod_uid)
    {
        return Err(StoreError::ConfigurationRevisionConflict);
    }
    Ok(())
}
async fn authorized(
    tx: &Transaction<'_>,
    b: &Binding,
) -> Result<(Operation, StageAttempt), StoreError> {
    let op = locked(tx, &b.operation_id).await?;
    let a = active_attempt(tx, &op).await?.ok_or(StoreError::NotFound)?;
    same_binding(&op, &a, b, true)?;
    if op.state != "running"
        || a.state != "bound"
        || op.stage != "cleanup" && now(tx).await? > a.deadline_unix_ms
        || op.stage != "cleanup" && !current(tx, &op).await?
    {
        return Err(StoreError::ConfigurationRevisionConflict);
    }
    Ok((op, a))
}
impl WorkflowStore {
    /// Save one bounded build checkpoint from the currently bound worker.
    /// # Errors
    /// Reject a stale binding or an unavailable diagnostic store.
    pub async fn record_runtime_build_checkpoint(
        &self,
        b: &Binding,
        milestone: BuildMilestone,
    ) -> Result<(), StoreError> {
        if b.stage != "build" {
            return Err(StoreError::InvalidInput("runtime build checkpoint"));
        }
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        authorized(&tx, b).await?;
        let mut record = Diagnostic::new(
            Kind::Runtime,
            &b.operation_id,
            b.generation,
            &b.stage,
            "build_checkpoint",
        );
        record.pod_uid.clone_from(&b.pod_uid);
        record.outcome = match milestone {
            BuildMilestone::CopySource => "copy_source",
            BuildMilestone::VerifyInputs => "verify_inputs",
            BuildMilestone::BuildImage => "build_image",
            BuildMilestone::ExportImage => "export_image",
            BuildMilestone::InspectImage => "inspect_image",
            BuildMilestone::ImageReady => "image_ready",
        }
        .into();
        crate::store::execution_diagnostics::append(&tx, &record).await?;
        tx.commit().await?;
        driver.abort();
        Ok(())
    }
    /// Claim or replay one exact retained stage, including existing pod adoption.
    /// # Errors
    /// Reject terminal/blocked operations, exhausted budgets and unavailable state.
    pub async fn claim_runtime_stage(&self, id: &str) -> Result<StageClaim, StoreError> {
        validate_id(id, "operation_id")?;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        let mut op = locked(&tx, id).await?;
        let at = now(&tx).await?;
        if !matches!(
            op.state.as_str(),
            "pending" | "running" | "stopping" | "cancelling" | "cleanup_pending"
        ) {
            return Err(StoreError::InvalidInput(
                "runtime operation is not schedulable",
            ));
        }
        let active = active_attempt(&tx, &op).await?;
        if op.stage != "cleanup" && (at > op.deadline_unix_ms || !current(&tx, &op).await?) {
            let reason = if at > op.deadline_unix_ms {
                "deadline_exceeded"
            } else {
                "selection_superseded"
            };
            let terminal = if at > op.deadline_unix_ms {
                "failed"
            } else {
                "superseded"
            };
            tx.execute("UPDATE workflow_runtime_preparations SET state='cancelling',terminal_state=$2,safe_reason=$3 WHERE id=$1",&[&id,&terminal,&reason]).await?;
            op = locked(&tx, id).await?;
        }
        if let Some(attempt) = active {
            tx.commit().await?;
            driver.abort();
            return Ok(StageClaim {
                operation: op,
                attempt,
            });
        }
        if op.state == "cancelling" {
            tx.execute("UPDATE workflow_runtime_preparations SET stage='cleanup',state='cleanup_pending',generation=generation+1 WHERE id=$1",&[&id]).await?;
            op = locked(&tx, id).await?;
        }
        reserve_build(&tx, &mut op).await?;
        let command_digest = digest(json(&(
            &op.frozen_input_digest,
            &op.stage,
            op.generation,
            &op.evidence,
            op.builds_started,
        ))?);
        let deadline = if op.stage == "cleanup" {
            at.saturating_add(300_000)
        } else {
            op.deadline_unix_ms
        };
        let row=tx.query_one("INSERT INTO workflow_runtime_preparation_attempts(id,operation_id,generation,stage,command_digest,state,deadline_unix_ms) VALUES($1,$2,$3,$4,$5,'claimed',$6) RETURNING *",&[&new_id(),&id,&integer(op.generation)?,&op.stage,&command_digest,&integer(deadline)?]).await?;
        let result = StageClaim {
            operation: op,
            attempt: attempt(&row)?,
        };
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }
    /// Retain a known setup failure only after trusted supervision confirms no Pod exists.
    /// # Errors
    /// Reject bound/stale attempts, unsupported diagnostics and conflicting retries.
    pub async fn fail_unbound_runtime_stage(
        &self,
        b: &Binding,
        reason: &str,
    ) -> Result<Operation, StoreError> {
        validate_unbound_failure(b, reason)?;
        let report = StageReport {
            binding: b.clone(),
            outcome: "failed".into(),
            safe_reason: reason.into(),
            evidence: Evidence::default(),
        };
        let report_json = json(&report)?;
        let report_digest = digest(&report_json);
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        let op = locked(&tx, &b.operation_id).await?;
        let row=tx.query_opt("SELECT * FROM workflow_runtime_preparation_attempts WHERE operation_id=$1 AND generation=$2 FOR UPDATE",&[&b.operation_id,&integer(b.generation)?]).await?.ok_or(StoreError::NotFound)?;
        let a = attempt(&row)?;
        validate_unbound_attempt(&a, b)?;
        if !a.report_digest.is_empty() {
            if a.report_digest != report_digest || a.state != "stopped" {
                return Err(StoreError::IdempotencyConflict);
            }
            tx.commit().await?;
            driver.abort();
            return Ok(op);
        }
        same_binding(&op, &a, b, false)?;
        if a.state != "claimed" || !matches!(op.state.as_str(), "pending" | "cleanup_pending") {
            return Err(StoreError::ConfigurationRevisionConflict);
        }
        let row = tx
            .query_one(
                "SELECT terminal_state FROM workflow_runtime_preparations WHERE id=$1",
                &[&op.id],
            )
            .await?;
        let terminal: String = row.get("terminal_state");
        let mut transition = transitions::after_stop(&op, Some(&report), &terminal)?;
        if op.stage == "cleanup" {
            transition.reason = reason.into();
        }
        retain_stage_diagnostic(&tx, &report, None).await?;
        tx.execute("UPDATE workflow_runtime_preparation_attempts SET state='stopped',report_digest=$2,report=$3 WHERE id=$1",&[&a.id,&report_digest,&report_json]).await?;
        tx.execute("UPDATE workflow_runtime_preparations SET state=$2,stage=$3,terminal_state=$4,safe_reason=$5,generation=generation+1,updated_at_unix_ms=$6 WHERE id=$1",&[&op.id,&transition.state,&transition.stage,&transition.terminal,&transition.reason,&integer(now(&tx).await?)?]).await?;
        let result = locked(&tx, &op.id).await?;
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }
    /// Bind only the exact observed pod UID; a different pod cannot adopt a generation.
    /// # Errors
    /// Reject stale bindings, changed pod identity and unavailable state.
    pub async fn bind_runtime_stage(&self, b: &Binding) -> Result<(), StoreError> {
        let uid =
            uuid::Uuid::parse_str(&b.pod_uid).map_err(|_| StoreError::InvalidInput("pod_uid"))?;
        if uid.to_string() != b.pod_uid {
            return Err(StoreError::InvalidInput("pod_uid"));
        }
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        let op = locked(&tx, &b.operation_id).await?;
        let a = active_attempt(&tx, &op)
            .await?
            .ok_or(StoreError::NotFound)?;
        same_binding(&op, &a, b, false)?;
        if a.pod_uid.is_empty() {
            if !matches!(op.state.as_str(), "pending" | "cleanup_pending")
                || a.state != "claimed"
                || op.stage != "cleanup" && now(&tx).await? > a.deadline_unix_ms
                || op.stage != "cleanup" && !current(&tx, &op).await?
            {
                return Err(StoreError::ConfigurationRevisionConflict);
            }
            tx.execute("UPDATE workflow_runtime_preparation_attempts SET pod_uid=$3,state='bound' WHERE operation_id=$1 AND generation=$2",&[&op.id,&integer(op.generation)?,&b.pod_uid]).await?;
            tx.execute(
                "UPDATE workflow_runtime_preparations SET state='running' WHERE id=$1",
                &[&op.id],
            )
            .await?;
        } else if a.pod_uid != b.pod_uid {
            return Err(StoreError::ConfigurationRevisionConflict);
        }
        tx.commit().await?;
        driver.abort();
        Ok(())
    }
    /// Return inputs only to the current bound live workload.
    /// # Errors
    /// Reject stale scope, deadline, capability revisions or workload identity.
    pub async fn runtime_stage_input(&self, b: &Binding) -> Result<StageClaim, StoreError> {
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        let (operation, attempt) = authorized(&tx, b).await?;
        tx.commit().await?;
        driver.abort();
        Ok(StageClaim { operation, attempt })
    }
    /// Persist the final manifest expectation before any registry mutation.
    /// # Errors
    /// Reject non-publication workloads, conflicting identities and stale capabilities.
    pub async fn worker_prepare_publication(
        &self,
        b: &Binding,
        expected: &str,
    ) -> Result<(), StoreError> {
        if !validation::sha(expected) {
            return Err(StoreError::InvalidInput("expected manifest digest"));
        }
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        let (mut op, _) = authorized(&tx, b).await?;
        if op.stage != "publish"
            || !op.evidence.expected_manifest_digest.is_empty()
                && op.evidence.expected_manifest_digest != expected
        {
            return Err(StoreError::ConfigurationRevisionConflict);
        }
        op.evidence.expected_manifest_digest = expected.into();
        tx.execute(
            "UPDATE workflow_runtime_preparations SET evidence=$2 WHERE id=$1",
            &[&op.id, &json(&op.evidence)?],
        )
        .await?;
        tx.commit().await?;
        driver.abort();
        Ok(())
    }
    /// Retain one immutable report. Stage advancement waits for confirmed pod deletion.
    /// # Errors
    /// Reject forged/stale reports, invalid stage evidence or conflicting retries.
    pub async fn report_runtime_stage(
        &self,
        report: &StageReport,
    ) -> Result<Operation, StoreError> {
        self.report_runtime_stage_with_timings(report, [None; BUILD_STEPS.len()])
            .await
    }

    /// Retain control-plane measurements alongside the first accepted build report.
    /// # Errors
    /// Reject forged/stale reports, invalid stage evidence or conflicting retries.
    pub async fn report_runtime_stage_with_timings(
        &self,
        report: &StageReport,
        elapsed_ms: [Option<u64>; BUILD_STEPS.len()],
    ) -> Result<Operation, StoreError> {
        validation::report(report)?;
        let diagnostic = certification_diagnostic(report)?;
        let report_digest = digest(json(report)?);
        let b = &report.binding;
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        let op = locked(&tx, &b.operation_id).await?;
        let old=tx.query_opt("SELECT * FROM workflow_runtime_preparation_attempts WHERE operation_id=$1 AND generation=$2 FOR UPDATE",&[&b.operation_id,&integer(b.generation)?]).await?.ok_or(StoreError::NotFound)?;
        let old = attempt(&old)?;
        if !old.report_digest.is_empty() {
            if old.report_digest != report_digest {
                return Err(StoreError::IdempotencyConflict);
            }
            tx.commit().await?;
            driver.abort();
            return Ok(op);
        }
        let (mut op, _) = authorized(&tx, b).await?;
        if diagnostic.is_some() {
            let mut report = report.clone();
            report.evidence.details.remove(CERTIFICATION_DIAGNOSTIC);
            transitions::accept_evidence(&mut op, &report)?;
        } else {
            transitions::accept_evidence(&mut op, report)?;
        }
        retain_stage_diagnostic(&tx, report, diagnostic).await?;
        if op.stage == "build" {
            op.evidence.details.insert(
                BUILD_REPORT_KEY.into(),
                json(&BuildReport {
                    complete: report.outcome == "completed",
                    elapsed_ms,
                })?,
            );
        }
        tx.execute("UPDATE workflow_runtime_preparation_attempts SET state='reported',report_digest=$3,report=$4 WHERE operation_id=$1 AND generation=$2",&[&op.id,&integer(op.generation)?,&report_digest,&json(report)?]).await?;
        tx.execute("UPDATE workflow_runtime_preparations SET state='stopping',evidence=$2,updated_at_unix_ms=$3 WHERE id=$1",&[&op.id,&json(&op.evidence)?,&integer(now(&tx).await?)?]).await?;
        let result = locked(&tx, &op.id).await?;
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }
    /// Confirm the exact workload is gone before advancing or completing cleanup.
    /// # Errors
    /// Reject wrong generations/UIDs and unavailable retained state.
    pub async fn stop_runtime_stage(&self, b: &Binding) -> Result<(), StoreError> {
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        let op = locked(&tx, &b.operation_id).await?;
        let old=tx.query_opt("SELECT * FROM workflow_runtime_preparation_attempts WHERE operation_id=$1 AND generation=$2 FOR UPDATE",&[&b.operation_id,&integer(b.generation)?]).await?.ok_or(StoreError::NotFound)?;
        let a = attempt(&old)?;
        if a.stage != b.stage || a.command_digest != b.command_digest || a.pod_uid != b.pod_uid {
            return Err(StoreError::ConfigurationRevisionConflict);
        }
        if a.state == "stopped" {
            tx.commit().await?;
            driver.abort();
            return Ok(());
        }
        same_binding(&op, &a, b, !a.pod_uid.is_empty())?;
        let row=tx.query_one("SELECT report,terminal_state FROM workflow_runtime_preparation_attempts a JOIN workflow_runtime_preparations p ON p.id=a.operation_id WHERE a.id=$1",&[&a.id]).await?;
        let report: String = row.get("report");
        let terminal: String = row.get("terminal_state");
        let report = if report.is_empty() {
            None
        } else {
            Some(
                serde_json::from_str::<StageReport>(&report)
                    .map_err(|_| StoreError::CorruptState)?,
            )
        };
        let transition = transitions::after_stop(&op, report.as_ref(), &terminal)?;
        reset_candidate_lineage(&tx, &op, &transition).await?;
        let mut state = transition.state;
        if state == "ready" && !current(&tx, &op).await? {
            state = "superseded".into();
        }
        tx.execute(
            "UPDATE workflow_runtime_preparation_attempts SET state='stopped' WHERE id=$1",
            &[&a.id],
        )
        .await?;
        tx.execute("UPDATE workflow_runtime_preparations SET stage=$2,state=$3,terminal_state=$4,safe_reason=$5,generation=generation+1,updated_at_unix_ms=$6 WHERE id=$1",&[&op.id,&transition.stage,&state,&transition.terminal,&transition.reason,&integer(now(&tx).await?)?]).await?;
        tx.commit().await?;
        driver.abort();
        Ok(())
    }
    /// Deliver node pull credentials only to the separately authenticated supervisor.
    /// # Errors
    /// Reject stale generations, non-verification stages, changed credentials or pod identities.
    pub async fn runtime_supervisor_pull_credential(
        &self,
        b: &Binding,
    ) -> Result<Credential, StoreError> {
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        let op = locked(&tx, &b.operation_id).await?;
        let a = active_attempt(&tx, &op)
            .await?
            .ok_or(StoreError::NotFound)?;
        same_binding(&op, &a, b, false)?;
        if !matches!(op.stage.as_str(), "verify_content" | "verify_compatibility")
            || !matches!(
                (op.state.as_str(), a.state.as_str()),
                ("pending", "claimed") | ("running", "bound")
            )
            || b.pod_uid != a.pod_uid
            || op.stage != "cleanup" && now(&tx).await? > a.deadline_unix_ms
            || !current(&tx, &op).await?
        {
            return Err(StoreError::ConfigurationRevisionConflict);
        }
        let registry = tx
            .query_one(
                "SELECT settings FROM workflow_connections WHERE id=$1",
                &[&op.frozen.registry_connection_id],
            )
            .await?;
        let settings: connections::ProviderSettings =
            serde_json::from_str(registry.get("settings")).map_err(|_| StoreError::CorruptState)?;
        let row=tx.query_one("SELECT value FROM workflow_connection_credentials WHERE connection_id=$1 AND revision_id=$2",&[&op.frozen.registry_connection_id,&op.frozen.registry_credential_revision]).await?;
        let tokens: Vec<String> =
            serde_json::from_str(row.get("value")).map_err(|_| StoreError::CorruptState)?;
        let password = tokens.get(1).ok_or(StoreError::CorruptState)?.clone();
        if tokens.len() != 2 {
            return Err(StoreError::CorruptState);
        }
        tx.commit().await?;
        driver.abort();
        Ok(Credential::Registry {
            username: settings.pull_username,
            password,
        })
    }
    /// Deliver a single stage's frozen source or GHCR capability; never signing material.
    /// # Errors
    /// Reject stale bindings, wrong stages and unavailable frozen credentials.
    pub async fn runtime_stage_credential(&self, b: &Binding) -> Result<Credential, StoreError> {
        let (mut client, driver) = crate::connect(&self.database_url).await?;
        let tx = client.transaction().await?;
        let (op, _) = authorized(&tx, b).await?;
        let f = &op.frozen;
        let result = if op.stage == "acquire_source" {
            let row=tx.query_one("SELECT value FROM workflow_connection_credentials WHERE revision_id=$1 AND connection_id=$2",&[&f.source_credential_revision,&f.source_connection_id]).await?;
            let source = tx
                .query_one(
                    "SELECT app_id FROM workflow_connections WHERE id=$1",
                    &[&f.source_connection_id],
                )
                .await?;
            Credential::Source {
                credential: super::super::repositories::SourceCredential {
                    app_id: source.get("app_id"),
                    private_key_pem: row.get("value"),
                },
                installation_id: f.installation_id,
                repository_id: f.provider_repository_id,
            }
        } else if matches!(
            op.stage.as_str(),
            "discover"
                | "pull_existing"
                | "publish"
                | "pull_final"
                | "verify_content"
                | "verify_compatibility"
        ) {
            let registry = tx
                .query_one(
                    "SELECT settings FROM workflow_connections WHERE id=$1",
                    &[&f.registry_connection_id],
                )
                .await?;
            let settings: connections::ProviderSettings =
                serde_json::from_str(registry.get("settings"))
                    .map_err(|_| StoreError::CorruptState)?;
            let row=tx.query_one("SELECT value FROM workflow_connection_credentials WHERE revision_id=$1 AND connection_id=$2",&[&f.registry_credential_revision,&f.registry_connection_id]).await?;
            let tokens: Vec<String> =
                serde_json::from_str(row.get("value")).map_err(|_| StoreError::CorruptState)?;
            if tokens.len() != 2 {
                return Err(StoreError::CorruptState);
            }
            let index = usize::from(op.stage != "publish");
            Credential::Registry {
                username: if index == 0 {
                    settings.push_username
                } else {
                    settings.pull_username
                },
                password: tokens[index].clone(),
            }
        } else {
            Credential::None
        };
        tx.commit().await?;
        driver.abort();
        Ok(result)
    }
}

fn certification_diagnostic(
    report: &StageReport,
) -> Result<Option<CertificationDiagnostic>, StoreError> {
    let Some(encoded) = report.evidence.details.get(CERTIFICATION_DIAGNOSTIC) else {
        return Ok(None);
    };
    if report.binding.stage != "certify" || report.outcome == "completed" {
        return Err(StoreError::InvalidInput(
            "runtime certification diagnostic scope",
        ));
    }
    serde_json::from_str(encoded)
        .map(Some)
        .map_err(|_| StoreError::InvalidInput("runtime certification diagnostic"))
}

async fn retain_stage_diagnostic(
    tx: &Transaction<'_>,
    report: &StageReport,
    cause: Option<CertificationDiagnostic>,
) -> Result<(), StoreError> {
    let b = &report.binding;
    let mut diagnostic = Diagnostic::new(
        Kind::Runtime,
        &b.operation_id,
        b.generation,
        &b.stage,
        "stage_report",
    );
    diagnostic.pod_uid.clone_from(&b.pod_uid);
    diagnostic.outcome.clone_from(&report.outcome);
    diagnostic.reason.clone_from(&report.safe_reason);
    if b.stage == "certify" && report.outcome != "completed" {
        diagnostic.certification = cause;
        if diagnostic.certification.is_none()
            && b.pod_uid.is_empty()
            && report.safe_reason == "certification_failed"
        {
            diagnostic.certification =
                Some(CertificationDiagnostic::SignerIdentityRejectedBeforeExecution);
        }
    }
    crate::store::execution_diagnostics::append(tx, &diagnostic).await
}

async fn reserve_build(tx: &Transaction<'_>, op: &mut Operation) -> Result<(), StoreError> {
    let id = op.id.clone();
    if op.stage == "build" {
        if !op.allow_build || op.builds_started >= op.max_builds {
            tx.execute("UPDATE workflow_runtime_preparations SET stage='cleanup',state='cleanup_pending',terminal_state='blocked',safe_reason='build_allowance_exhausted',generation=generation+1 WHERE id=$1",&[&id]).await?;
        } else {
            tx.execute("UPDATE workflow_runtime_preparations SET builds_started=builds_started+1 WHERE id=$1",&[&id]).await?;
        }
        *op = locked(tx, &id).await?;
    }
    Ok(())
}

async fn reset_candidate_lineage(
    tx: &Transaction<'_>,
    op: &Operation,
    transition: &transitions::Transition,
) -> Result<(), StoreError> {
    if transition.stage == "build"
        && matches!(op.stage.as_str(), "verify_content" | "discover")
        && op.evidence.expected_manifest_digest.is_empty()
    {
        let evidence = Evidence {
            source_digest: op.evidence.source_digest.clone(),
            ..Evidence::default()
        };
        tx.execute(
            "UPDATE workflow_runtime_preparations SET evidence=$2 WHERE id=$1",
            &[&op.id, &json(&evidence)?],
        )
        .await?;
    }
    Ok(())
}

fn validate_unbound_failure(b: &Binding, reason: &str) -> Result<(), StoreError> {
    if !b.pod_uid.is_empty()
        || !matches!(
            reason,
            "setup_failure" | "certification_failed" | "builder_unavailable"
        )
        || reason == "certification_failed" && b.stage != "certify"
        || reason == "builder_unavailable" && b.stage != "build"
    {
        return Err(StoreError::InvalidInput("unbound stage failure"));
    }
    Ok(())
}
fn validate_unbound_attempt(a: &StageAttempt, b: &Binding) -> Result<(), StoreError> {
    if !a.pod_uid.is_empty() || a.stage != b.stage || a.command_digest != b.command_digest {
        return Err(StoreError::ConfigurationRevisionConflict);
    }
    Ok(())
}
