use super::{
    CleanStore, CleanStoreError, Digest, Message, Sha256, decode, params,
    persist_change_request_update, request_digest, timestamp, unix_seconds, v1,
};

pub(super) type RetainedReviewEvidence = (Vec<u8>, std::collections::BTreeMap<String, Vec<u8>>);

impl CleanStore {
    pub(crate) fn retained_review_evidence(
        &self,
        assessment_id: &str,
    ) -> Result<Option<RetainedReviewEvidence>, CleanStoreError> {
        let assessment = match self.assessment(assessment_id) {
            Ok(value) => value,
            Err(CleanStoreError::NotFound { .. }) => return Ok(None),
            Err(error) => return Err(error),
        };
        let base = self
            .path
            .as_ref()
            .and_then(|p| p.parent())
            .ok_or(CleanStoreError::InvalidTransition)?;
        let mut artifacts = std::collections::BTreeMap::new();
        for id in &assessment.evidence_reference_ids {
            let evidence = self.evidence_reference(id)?;
            if evidence.assessment_id != assessment.id
                || !matches!(
                    evidence.name.as_str(),
                    "manifest.json" | "signals.json" | "runtime.json"
                )
            {
                return Err(CleanStoreError::ParentMismatch);
            }
            let path = base
                .join("assessments")
                .join(assessment.id.replace(':', "-"))
                .join(&evidence.name);
            let bytes =
                ayni_orchestrator_runtime::runtime_execution::read_bounded(&path, 10 * 1024 * 1024)
                    .map_err(CleanStoreError::CreateDirectory)?;
            if bytes.len() as u64 != evidence.size_bytes
                || ayni_orchestrator_domain::digest::sha256(&bytes) != evidence.digest
            {
                return Err(CleanStoreError::IdempotencyConflict);
            }
            artifacts.insert(evidence.name, bytes);
        }
        let manifest = artifacts
            .remove("manifest.json")
            .ok_or(CleanStoreError::CorruptState)?;
        Ok(Some((manifest, artifacts)))
    }

    pub(crate) fn publication_request_for(
        &self,
        change_id: &str,
    ) -> Result<v1::PublishCandidateRequest, CleanStoreError> {
        let bytes: Vec<u8> = self.connection.query_row("SELECT request_payload FROM durable_actions WHERE operation='PublishCandidate' AND target_id=?1 AND state='completed'", [change_id], |row| row.get(0)).map_err(CleanStoreError::Database)?;
        decode(&bytes)
    }

    pub(crate) fn verify_assessment_files(
        &self,
        assessment: &v1::Assessment,
    ) -> Result<(), CleanStoreError> {
        let base = self
            .path
            .as_ref()
            .and_then(|p| p.parent())
            .ok_or(CleanStoreError::InvalidTransition)?;
        for id in &assessment.evidence_reference_ids {
            let evidence = self.evidence_reference(id)?;
            if evidence.assessment_id != assessment.id {
                return Err(CleanStoreError::ParentMismatch);
            }
            let path = base
                .join("assessments")
                .join(assessment.id.replace(':', "-"))
                .join(&evidence.name);
            let bytes =
                ayni_orchestrator_runtime::runtime_execution::read_bounded(&path, 10 * 1024 * 1024)
                    .map_err(CleanStoreError::CreateDirectory)?;
            if bytes.len() as u64 != evidence.size_bytes
                || ayni_orchestrator_domain::digest::sha256(&bytes) != evidence.digest
            {
                return Err(CleanStoreError::IdempotencyConflict);
            }
        }
        if assessment.evidence_reference_ids.len() != 3 {
            return Err(CleanStoreError::InvalidTransition);
        }
        Ok(())
    }

    pub(crate) fn save_readiness(
        &mut self,
        id: &str,
        status: v1::ReadinessStatus,
        action: bool,
    ) -> Result<v1::ChangeRequest, CleanStoreError> {
        let mut change = self.change_request(id)?;
        let previous = change.resource_version;
        let key = readiness_key(id, &status.assessment_id);
        if action {
            let assessment = self.assessment(&status.assessment_id)?;
            if status.policy != "READY_AFTER_SUCCESSFUL_ASSESSMENT"
                || status.state != "pending"
                || change.state != v1::ChangeRequestState::Draft as i32
                || assessment.change_request_id != id
                || assessment.head_revision != change.head_revision
                || assessment.conclusion != v1::AssessmentConclusion::Succeeded as i32
                || status.authorized_head != assessment.head_revision
                || change.candidate_id.is_empty()
            {
                return Err(CleanStoreError::InvalidTransition);
            }
            self.verify_assessment_files(&assessment)?;
        }
        if change.readiness.as_ref() == Some(&status) {
            return Ok(change);
        }
        change.readiness = Some(status.clone());
        change.resource_version += 1;
        change.updated_at = Some(timestamp(unix_seconds()?));
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        if action {
            transaction.execute("INSERT OR IGNORE INTO durable_actions(operation,idempotency_key,request_hash,request_payload,state,target_type,target_id,created_at,updated_at) VALUES ('MarkReady',?1,?2,?3,'pending','change_request',?4,?5,?5)", params![key,request_digest("MarkReady",&status),status.encode_to_vec(),id,unix_seconds()?]).map_err(CleanStoreError::Database)?;
        }
        if status.state == "completed" {
            transaction.execute("UPDATE durable_actions SET state='completed',failure=NULL,updated_at=?1 WHERE operation='MarkReady' AND idempotency_key=?2", params![unix_seconds()?,key]).map_err(CleanStoreError::Database)?;
        }
        persist_change_request_update(&transaction, &change, previous)?;
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(change)
    }
}

pub(crate) fn readiness_key(change: &str, assessment: &str) -> String {
    format!(
        "ready:{:x}",
        Sha256::digest(format!("{change}:{assessment}"))
    )
}
