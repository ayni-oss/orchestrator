impl CleanStore {
    pub(crate) fn observe_review(
        &mut self,
        repository_id: &str,
        provider: v1::GithubChangeRequest,
        head: &str,
        base: &str,
        state: v1::ChangeRequestState,
    ) -> Result<v1::ChangeRequest, CleanStoreError> {
        if !valid_git_revision(head) || !valid_git_revision(base) {
            return Err(CleanStoreError::BoundExceeded);
        }
        let repository = self.repository_connection(repository_id)?;
        let Some(v1::repository_provider::Provider::Github(github)) =
            repository.provider.and_then(|p| p.provider)
        else {
            return Err(CleanStoreError::ParentMismatch);
        };
        if github.repository_id != provider.repository_id {
            return Err(CleanStoreError::ParentMismatch);
        }
        let mut statement = self
            .connection
            .prepare(
                "SELECT payload FROM change_requests WHERE repository_connection_id=?1 LIMIT 10001",
            )
            .map_err(CleanStoreError::Database)?;
        let mut rows = statement
            .query([repository_id])
            .map_err(CleanStoreError::Database)?;
        let changes = collect_bounded_payloads(&mut rows)?
            .iter()
            .map(|bytes| decode::<v1::ChangeRequest>(bytes))
            .collect::<Result<Vec<_>, _>>()?;
        drop(rows);
        drop(statement);
        let found = changes.into_iter().find(|c| c.provider.as_ref().and_then(|p| p.provider.as_ref()).is_some_and(|p| matches!(p, v1::change_request_provider::Provider::Github(g) if g.pull_request_number == provider.pull_request_number)));
        if let Some(mut change) = found {
            if change.head_revision != head
                || change.base_revision != base
                || change.state != state as i32
            {
                let previous = change.resource_version;
                change.head_revision = head.into();
                change.base_revision = base.into();
                change.state = state as i32;
                change.resource_version += 1;
                change.updated_at = Some(timestamp(unix_seconds()?));
                update_observed_readiness(&mut change, head, state);
                let transaction = self
                    .connection
                    .transaction()
                    .map_err(CleanStoreError::Database)?;
                if change
                    .readiness
                    .as_ref()
                    .is_some_and(|r| r.state == "obsolete")
                {
                    transaction.execute("UPDATE durable_actions SET state='failed',failure='head_changed',updated_at=?1 WHERE operation='MarkReady' AND target_id=?2 AND state='pending'",params![unix_seconds()?,change.id]).map_err(CleanStoreError::Database)?;
                }
                persist_change_request_update(&transaction, &change, previous)?;
                transaction.commit().map_err(CleanStoreError::Database)?;
            }
            return Ok(change);
        }
        let id = format!(
            "change-request:{:x}",
            Sha256::digest(format!("{repository_id}:{}", provider.pull_request_number))
        );
        let change = v1::ChangeRequest {
            id: id.clone(),
            repository_connection_id: repository_id.into(),
            provider: Some(v1::ChangeRequestProvider {
                provider: Some(v1::change_request_provider::Provider::Github(provider)),
            }),
            head_revision: head.into(),
            base_revision: base.into(),
            state: state as i32,
            resource_version: 1,
            created_at: Some(timestamp(unix_seconds()?)),
            updated_at: Some(timestamp(unix_seconds()?)),
            ..Default::default()
        };
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let event = append_event(
            &transaction,
            REVIEW_SERVICE,
            "change_request",
            &id,
            1,
            Some(repository_id),
            Some(&id),
            &change,
        )?;
        transaction.execute("INSERT INTO change_requests(id,repository_connection_id,candidate_id,state,head_revision,payload,resource_version,event_cursor) VALUES (?1,?2,NULL,?3,?4,?5,1,?6)", params![id,repository_id,change.state,head,change.encode_to_vec(),i64::try_from(event).map_err(|_| CleanStoreError::CorruptState)?]).map_err(CleanStoreError::Database)?;
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(change)
    }

    pub(crate) fn import_review(
        &mut self,
        change: &v1::ChangeRequest,
        manifest: &ayni_orchestrator_runtime::assessment::Manifest,
        summary: &ayni_orchestrator_runtime::assessment::Summary,
        manifest_bytes: &[u8],
        artifacts: &std::collections::BTreeMap<String, Vec<u8>>,
        workflow_url: &str,
    ) -> Result<v1::Assessment, CleanStoreError> {
        use ayni_orchestrator_runtime::assessment::{Outcome, sha256};
        let identity = format!(
            "{}:{}:{}",
            manifest.run.repository_id, manifest.run.run_id, manifest.run.attempt
        );
        let id = format!("assessment:{:x}", Sha256::digest(identity));
        let manifest_digest = sha256(manifest_bytes);
        if let Ok(existing) = self.assessment(&id) {
            if matches!(existing.details.as_ref(), Some(v1::assessment::Details::AyniManaged(d)) if d.manifest_digest == manifest_digest)
            {
                return Ok(existing);
            }
            return Err(CleanStoreError::IdempotencyConflict);
        }
        let revision = self.runtime_profile_revision(&manifest.run.profile_revision)?;
        let profile = self.runtime_profile(&revision.runtime_profile_id)?;
        if profile.repository_connection_id != change.repository_connection_id {
            return Err(CleanStoreError::ParentMismatch);
        }
        let conclusion = match manifest.outcome {
            Outcome::Succeeded => v1::AssessmentConclusion::Succeeded,
            Outcome::QualityFailed => v1::AssessmentConclusion::QualityFailed,
            Outcome::Incomplete => v1::AssessmentConclusion::Incomplete,
            Outcome::PolicyRejected => v1::AssessmentConclusion::PolicyRejected,
            Outcome::Unsupported => v1::AssessmentConclusion::Unsupported,
            Outcome::Stale => v1::AssessmentConclusion::Stale,
            Outcome::Cancelled => v1::AssessmentConclusion::Cancelled,
            Outcome::InfrastructureFailed => v1::AssessmentConclusion::InfrastructureFailed,
        };
        let mut retained = artifacts.clone();
        retained.insert("manifest.json".into(), manifest_bytes.to_vec());
        retain_assessment_files(self.path.as_deref(), &id, &retained)?;
        let evidence: Vec<_> = retained
            .iter()
            .enumerate()
            .map(|(index, (name, bytes))| v1::EvidenceReference {
                id: format!("evidence:{}:{index}", id.trim_start_matches("assessment:")),
                assessment_id: id.clone(),
                name: name.clone(),
                digest: sha256(bytes),
                media_type: "application/json".into(),
                size_bytes: bytes.len() as u64,
                verified_at: Some(timestamp(unix_seconds().unwrap_or(0))),
                resource_version: 1,
                ..Default::default()
            })
            .collect();
        let assessment = v1::Assessment {
            id: id.clone(),
            repository_connection_id: change.repository_connection_id.clone(),
            change_request_id: change.id.clone(),
            head_revision: manifest.run.head.clone(),
            runtime_profile_revision_id: manifest.run.profile_revision.clone(),
            producer: Some(v1::AssessmentProducer {
                kind: v1::AssessmentProducerKind::AyniManaged as i32,
                name: "trusted-actions-collector".into(),
                version: manifest.version.clone(),
            }),
            workflow: Some(v1::WorkflowExecution {
                provider: Some(v1::workflow_execution::Provider::Github(
                    v1::GithubWorkflowExecution {
                        workflow_id: manifest.run.workflow_id,
                        run_id: manifest.run.run_id,
                        attempt: u32::try_from(manifest.run.attempt)
                            .map_err(|_| CleanStoreError::BoundExceeded)?,
                        event_name: "workflow_run".into(),
                        head_sha: manifest.run.workflow_sha.clone(),
                        html_url: workflow_url.into(),
                    },
                )),
            }),
            conclusion: conclusion as i32,
            details: Some(v1::assessment::Details::AyniManaged(
                v1::AyniManagedAssessment {
                    lock_fingerprint: manifest
                        .lock_fingerprint
                        .trim_start_matches("sha256:")
                        .into(),
                    runtime_digest: manifest.runtime_digest.clone(),
                    manifest_digest,
                    completed_target_count: summary.completed,
                    skipped_target_count: summary.skipped,
                    warning_count: summary.warnings,
                    failure_count: summary.failures,
                },
            )),
            evidence_reference_ids: evidence.iter().map(|e| e.id.clone()).collect(),
            created_at: Some(timestamp(unix_seconds()?)),
            resource_version: 1,
        };
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let cursor = append_event(
            &transaction,
            REVIEW_SERVICE,
            "assessment",
            &id,
            1,
            Some(&change.repository_connection_id),
            Some(&change.id),
            &assessment,
        )?;
        transaction.execute("INSERT INTO assessments(id,repository_connection_id,change_request_id,head_revision,conclusion,payload,resource_version,event_cursor) VALUES (?1,?2,?3,?4,?5,?6,1,?7)",params![id,change.repository_connection_id,change.id,assessment.head_revision,assessment.conclusion,assessment.encode_to_vec(),i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?]).map_err(CleanStoreError::Database)?;
        for evidence in evidence {
            let cursor = append_event(
                &transaction,
                REVIEW_SERVICE,
                "evidence_reference",
                &evidence.id,
                1,
                Some(&change.repository_connection_id),
                Some(&change.id),
                &evidence,
            )?;
            transaction.execute("INSERT INTO evidence_references(id,assessment_id,payload,resource_version,event_cursor) VALUES (?1,?2,?3,1,?4)",params![evidence.id,id,evidence.encode_to_vec(),i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?]).map_err(CleanStoreError::Database)?;
        }
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(assessment)
    }
}

fn update_observed_readiness(change: &mut v1::ChangeRequest, head: &str, state: v1::ChangeRequestState) {
                if let Some(readiness) = &mut change.readiness {
                    if !readiness.authorized_head.is_empty() && readiness.authorized_head != head {
                        readiness.state = "obsolete".into();
                        readiness.diagnostic = "new_head_requires_assessment".into();
                    } else if state == v1::ChangeRequestState::Open
                        && readiness.state != "completed"
                        && readiness.state != "pending"
                    {
                        readiness.source = "maintainer".into();
                        readiness.state = "manual".into();
                    }
                }
}

fn retain_assessment_files(database: Option<&std::path::Path>, id: &str, retained: &std::collections::BTreeMap<String, Vec<u8>>) -> Result<(), CleanStoreError> {
        // A separate directory next to the controller database is never mounted
        // into coding or assessment containers. create_new + byte comparison
        // makes a crash between retention and commit safely replayable.
        if let Some(path) = database {
            let directory = path
                .parent()
                .ok_or(CleanStoreError::CorruptState)?
                .join("assessments")
                .join(id.replace(':', "-"));
            fs::create_dir_all(&directory).map_err(CleanStoreError::CreateDirectory)?;
            for (name, bytes) in retained {
                use std::io::Write;
                let path = directory.join(name);
                let mut temporary = tempfile::NamedTempFile::new_in(&directory)
                    .map_err(CleanStoreError::CreateDirectory)?;
                temporary
                    .write_all(bytes)
                    .and_then(|()| temporary.as_file().sync_all())
                    .map_err(CleanStoreError::CreateDirectory)?;
                match temporary.persist_noclobber(&path) {
                    Ok(_) => {}
                    Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                        if fs::read(&path).map_err(CleanStoreError::CreateDirectory)? != *bytes {
                            return Err(CleanStoreError::IdempotencyConflict);
                        }
                    }
                    Err(error) => return Err(CleanStoreError::CreateDirectory(error.error)),
                }
            }
        }
    Ok(())
}
