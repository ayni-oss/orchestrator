//! Transactional adapter for application-owned publication admission.
use super::{
    CleanStore, CleanStoreError, Message, REVIEW_SERVICE, action_row, append_event, map_constraint,
    params, resource_id, timestamp, unix_seconds, v1,
};
use ayni_orchestrator_application::{
    publication::{CandidateTarget, PublicationError, PublicationIntent, Repository},
    tasks::RequestIdentity,
};

pub(crate) fn publication_error(error: PublicationError<CleanStoreError>) -> CleanStoreError {
    match error {
        PublicationError::Storage(error) => error,
        PublicationError::NotPublishable => CleanStoreError::InvalidTransition,
    }
}

impl Repository for CleanStore {
    type Error = CleanStoreError;

    fn publication_replay(
        &self,
        identity: &RequestIdentity,
    ) -> Result<Option<String>, Self::Error> {
        let Some(existing) = action_row(&self.connection, "PublishCandidate", &identity.key)?
        else {
            return Ok(None);
        };
        if existing.request_hash != identity.digest {
            return Err(CleanStoreError::IdempotencyConflict);
        }
        Ok(Some(
            existing.target_id.ok_or(CleanStoreError::CorruptState)?,
        ))
    }

    fn publication_target(&self, candidate_id: &str) -> Result<CandidateTarget, Self::Error> {
        let candidate = self.candidate(candidate_id)?;
        let git_head = candidate
            .identity
            .as_ref()
            .and_then(|i| i.identity.as_ref())
            .and_then(|i| match i {
                v1::candidate_identity::Identity::Git(git) => Some(git.commit_sha.clone()),
                v1::candidate_identity::Identity::Bundle(_) => None,
            });
        Ok(CandidateTarget {
            candidate_id: candidate.id,
            candidate_version: candidate.resource_version,
            repository_id: candidate.repository_connection_id,
            base_revision: candidate.base_revision,
            git_head,
            available: candidate.state == v1::CandidateState::Available as i32,
        })
    }

    fn commit_publication(
        &mut self,
        identity: RequestIdentity,
        intent: PublicationIntent,
    ) -> Result<String, Self::Error> {
        if self.candidate(&intent.candidate_id)?.resource_version != intent.candidate_version {
            return Err(CleanStoreError::StaleVersion);
        }
        let now = unix_seconds()?;
        let id = resource_id("change-request");
        let change_request = v1::ChangeRequest {
            readiness: None,
            id: id.clone(),
            repository_connection_id: intent.repository_id,
            candidate_id: intent.candidate_id,
            provider: None,
            base_revision: intent.base_revision,
            head_revision: intent.head_revision,
            state: v1::ChangeRequestState::Publishing as i32,
            failure: None,
            created_at: Some(timestamp(now)),
            updated_at: Some(timestamp(now)),
            resource_version: 1,
        };
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        transaction
            .execute(
                "INSERT INTO durable_actions(
                    operation,idempotency_key,request_hash,request_payload,state,target_type,target_id,created_at,updated_at
                 ) VALUES ('PublishCandidate',?1,?2,?3,'pending','change_request',?4,?5,?5)",
                params![identity.key, identity.digest, identity.payload, id, now],
            )
            .map_err(map_constraint)?;
        let cursor = append_event(
            &transaction,
            REVIEW_SERVICE,
            "change_request",
            &id,
            1,
            Some(&change_request.repository_connection_id),
            Some(&id),
            &change_request,
        )?;
        transaction
            .execute(
                "INSERT INTO change_requests(
                    id,repository_connection_id,candidate_id,state,head_revision,payload,
                    resource_version,event_cursor
                 ) VALUES (?1,?2,?3,?4,?5,?6,1,?7)",
                params![
                    id,
                    change_request.repository_connection_id,
                    change_request.candidate_id,
                    change_request.state,
                    change_request.head_revision,
                    change_request.encode_to_vec(),
                    i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(map_constraint)?;
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(change_request.id)
    }
}

pub(crate) fn publication_command(
    key: &str,
    request: &v1::PublishCandidateRequest,
) -> ayni_orchestrator_application::publication::PublishCandidate {
    ayni_orchestrator_application::publication::PublishCandidate {
        candidate_id: request.candidate_id.clone(),
        identity: RequestIdentity {
            key: key.into(),
            digest: super::request_digest("PublishCandidate", request),
            payload: request.encode_to_vec(),
        },
    }
}

impl ayni_orchestrator_application::publication::AutomaticRepository for CleanStore {
    fn automatic_targets(
        &self,
    ) -> Result<Vec<ayni_orchestrator_application::publication::AutomaticTarget>, Self::Error> {
        use ayni_orchestrator_application::publication::AutomaticTarget;
        let mut statement = self.connection.prepare("SELECT c.payload FROM candidates c LEFT JOIN change_requests r ON r.candidate_id=c.id WHERE r.id IS NULL AND c.state=1 LIMIT 10001")
            .map_err(CleanStoreError::Database)?;
        let mut rows = statement.query([]).map_err(CleanStoreError::Database)?;
        let candidates = super::collect_bounded_payloads(&mut rows)?;
        candidates
            .into_iter()
            .map(|bytes| {
                let candidate: v1::Candidate = super::decode(&bytes)?;
                let attempt = self.development_attempt(&candidate.development_attempt_id)?;
                let task = self.development_task(&attempt.development_task_id)?;
                Ok(AutomaticTarget {
                    candidate_id: candidate.id,
                    attempt_state: super::task_cancellation::attempt_state(attempt.state)?,
                    task_state: super::task_cancellation::task_state(task.state)?,
                    publication_base_branch: task
                        .snapshot
                        .filter(|s| s.publish_draft)
                        .map(|s| s.publication_base_branch),
                })
            })
            .collect()
    }

    fn automatic_identity(
        &self,
        request: ayni_orchestrator_application::publication::AutomaticRequest,
    ) -> RequestIdentity {
        publication_command(
            &request.key,
            &v1::PublishCandidateRequest {
                idempotency_key: request.key.clone(),
                candidate_id: request.candidate_id,
                title: request.title,
                body: request.body,
                base_branch: request.base_branch,
                draft: true,
            },
        )
        .identity
    }
}
