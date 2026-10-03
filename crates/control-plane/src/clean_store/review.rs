impl CleanStore {
    pub(crate) fn record_candidate(
        &mut self,
        mut candidate: v1::Candidate,
    ) -> Result<v1::Candidate, CleanStoreError> {
        candidate.failure = bounded_failure(candidate.failure);
        if !valid_candidate_report(&candidate) {
            return Err(CleanStoreError::BoundExceeded);
        }
        let existing: Option<Vec<u8>> = self
            .connection
            .query_row(
                "SELECT payload FROM candidates WHERE development_attempt_id=?1",
                [&candidate.development_attempt_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(CleanStoreError::Database)?;
        if let Some(payload) = existing {
            let existing = decode::<v1::Candidate>(&payload)?;
            return if same_candidate_report(&existing, &candidate) {
                Ok(existing)
            } else {
                Err(CleanStoreError::IdempotencyConflict)
            };
        }
        let attempt = self.development_attempt(&candidate.development_attempt_id)?;
        if attempt.state != v1::DevelopmentAttemptState::Succeeded as i32 {
            return Err(CleanStoreError::InvalidTransition);
        }
        let task = self.development_task(&attempt.development_task_id)?;
        let repository_id = task
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.repository.as_ref())
            .map(|repository| repository.repository_connection_id.clone())
            .ok_or(CleanStoreError::CorruptState)?;
        if candidate.repository_connection_id != repository_id {
            return Err(CleanStoreError::ParentMismatch);
        }
        if candidate.id.is_empty() {
            candidate.id = resource_id("candidate");
        }
        candidate.resource_version = 1;
        if candidate.created_at.is_none() {
            candidate.created_at = Some(timestamp(unix_seconds()?));
        }
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let cursor = append_event(
            &transaction,
            DEVELOPMENT_SERVICE,
            "candidate",
            &candidate.id,
            1,
            Some(&repository_id),
            Some(&task.id),
            &candidate,
        )?;
        transaction
            .execute(
                "INSERT INTO candidates(
                    id,development_task_id,development_attempt_id,repository_connection_id,
                    state,payload,resource_version,event_cursor
                 ) VALUES (?1,?2,?3,?4,?5,?6,1,?7)",
                params![
                    candidate.id,
                    task.id,
                    attempt.id,
                    repository_id,
                    candidate.state,
                    candidate.encode_to_vec(),
                    i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(map_constraint)?;
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(candidate)
    }

    pub(crate) fn candidate(&self, id: &str) -> Result<v1::Candidate, CleanStoreError> {
        get_payload(
            &self.connection,
            "SELECT payload FROM candidates WHERE id=?1",
            id,
            "candidate",
        )
    }

    pub(crate) fn reject_candidate(
        &mut self,
        id: &str,
        expected_version: u64,
        failure: v1::ResourceFailure,
    ) -> Result<v1::Candidate, CleanStoreError> {
        let mut resource = self.candidate(id)?;
        if resource.resource_version != expected_version {
            return Err(CleanStoreError::StaleVersion);
        }
        let current = v1::CandidateState::try_from(resource.state)
            .map_err(|_| CleanStoreError::CorruptState)?;
        if !valid_candidate_transition(current, v1::CandidateState::Rejected) {
            return Err(CleanStoreError::InvalidTransition);
        }
        let task_id: String = self
            .connection
            .query_row(
                "SELECT development_task_id FROM candidates WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .map_err(CleanStoreError::Database)?;
        let task = self.development_task(&task_id)?;
        let repository_id = task
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.repository.as_ref())
            .map(|repository| repository.repository_connection_id.as_str())
            .ok_or(CleanStoreError::CorruptState)?;
        resource.state = v1::CandidateState::Rejected as i32;
        resource.failure = bounded_failure(Some(failure));
        resource.resource_version = resource.resource_version.saturating_add(1);
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let cursor = append_event(
            &transaction,
            DEVELOPMENT_SERVICE,
            "candidate",
            &resource.id,
            resource.resource_version,
            Some(repository_id),
            Some(&task_id),
            &resource,
        )?;
        let changed = transaction
            .execute(
                "UPDATE candidates SET state=?1,payload=?2,resource_version=?3,event_cursor=?4
             WHERE id=?5 AND resource_version=?6",
                params![
                    resource.state,
                    resource.encode_to_vec(),
                    i64::try_from(resource.resource_version)
                        .map_err(|_| CleanStoreError::CorruptState)?,
                    i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?,
                    resource.id,
                    i64::try_from(expected_version).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(CleanStoreError::Database)?;
        if changed != 1 {
            return Err(CleanStoreError::StaleVersion);
        }
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(resource)
    }

    pub(crate) fn list_candidates(
        &mut self,
        task_id: Option<&str>,
        attempt_id: Option<&str>,
        page: Option<&v1::PageRequest>,
    ) -> Result<Page<v1::Candidate>, CleanStoreError> {
        let filter = encode_filter(&[task_id.unwrap_or(""), attempt_id.unwrap_or("")]);
        self.page_messages(
            "ListCandidates",
            &filter,
            page,
            |connection| {
                let mut statement = connection
                    .prepare(
                        "SELECT payload FROM candidates
                         WHERE (?1 IS NULL OR development_task_id=?1)
                           AND (?2 IS NULL OR development_attempt_id=?2)
                         ORDER BY id",
                    )
                    .map_err(CleanStoreError::Database)?;
                let mut rows = statement
                    .query(params![task_id, attempt_id])
                    .map_err(CleanStoreError::Database)?;
                collect_bounded_payloads(&mut rows)
            },
            DEVELOPMENT_SERVICE,
        )
    }

    pub(crate) fn publish_candidate(
        &mut self,
        key: &str,
        request: &v1::PublishCandidateRequest,
    ) -> Result<v1::ChangeRequest, CleanStoreError> {
        let digest = request_digest("PublishCandidate", request);
        if let Some(existing) = action_row(&self.connection, "PublishCandidate", key)? {
            if existing.request_hash != digest {
                return Err(CleanStoreError::IdempotencyConflict);
            }
            return self.change_request(&existing.target_id.ok_or(CleanStoreError::CorruptState)?);
        }
        let candidate = self.candidate(&request.candidate_id)?;
        if candidate.state != v1::CandidateState::Available as i32 {
            return Err(CleanStoreError::InvalidTransition);
        }
        let head_revision = match candidate
            .identity
            .as_ref()
            .and_then(|value| value.identity.as_ref())
        {
            Some(v1::candidate_identity::Identity::Git(git)) => git.commit_sha.clone(),
            _ => return Err(CleanStoreError::InvalidTransition),
        };
        let now = unix_seconds()?;
        let id = resource_id("change-request");
        let change_request = v1::ChangeRequest {
            readiness: None,
            id: id.clone(),
            repository_connection_id: candidate.repository_connection_id.clone(),
            candidate_id: candidate.id,
            provider: None,
            base_revision: candidate.base_revision,
            head_revision,
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
                params![key, digest, request.encode_to_vec(), id, now],
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
        Ok(change_request)
    }

    pub(crate) fn change_request(&self, id: &str) -> Result<v1::ChangeRequest, CleanStoreError> {
        get_payload(
            &self.connection,
            "SELECT payload FROM change_requests WHERE id=?1",
            id,
            "change_request",
        )
    }

    pub(crate) fn reconcile_change_request(
        &mut self,
        operation_key: &str,
        expected_version: u64,
        provider: v1::ChangeRequestProvider,
        state: v1::ChangeRequestState,
        head_revision: &str,
        failure: Option<v1::ResourceFailure>,
    ) -> Result<v1::ChangeRequest, CleanStoreError> {
        let action = self
            .durable_action("PublishCandidate", operation_key)?
            .ok_or(CleanStoreError::InvalidTransition)?;
        let id = action.target_id.ok_or(CleanStoreError::CorruptState)?;
        let mut change_request = self.change_request(&id)?;
        let current = v1::ChangeRequestState::try_from(change_request.state)
            .map_err(|_| CleanStoreError::CorruptState)?;
        let failure = bounded_failure(failure);
        if current == state
            && change_request.provider.as_ref() == Some(&provider)
            && change_request.head_revision == head_revision
            && equivalent_failure(change_request.failure.as_ref(), failure.as_ref())
            && (change_request.resource_version == expected_version
                || change_request.resource_version == expected_version.saturating_add(1))
        {
            return Ok(change_request);
        }
        if change_request.resource_version != expected_version {
            return Err(CleanStoreError::StaleVersion);
        }
        if !valid_change_request_transition(current, state) {
            return Err(CleanStoreError::InvalidTransition);
        }
        let previous_version = change_request.resource_version;
        change_request.provider = Some(provider);
        change_request.state = state as i32;
        head_revision.clone_into(&mut change_request.head_revision);
        change_request.failure = failure;
        change_request.updated_at = Some(timestamp(unix_seconds()?));
        change_request.resource_version = previous_version.saturating_add(1);
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        persist_change_request_update(&transaction, &change_request, previous_version)?;
        transaction
            .execute(
                "UPDATE durable_actions SET state=?1,failure=?2,updated_at=?3
                 WHERE operation='PublishCandidate' AND idempotency_key=?4",
                params![
                    if state == v1::ChangeRequestState::Failed {
                        "failed"
                    } else {
                        "completed"
                    },
                    change_request
                        .failure
                        .as_ref()
                        .map(|value| truncate_utf8(&value.message, 512)),
                    unix_seconds()?,
                    operation_key
                ],
            )
            .map_err(CleanStoreError::Database)?;
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(change_request)
    }

    pub(crate) fn retry_change_request_publication(
        &mut self,
        operation_key: &str,
        expected_version: u64,
    ) -> Result<v1::ChangeRequest, CleanStoreError> {
        let action = self
            .durable_action("PublishCandidate", operation_key)?
            .ok_or(CleanStoreError::InvalidTransition)?;
        if action.state != DurableActionState::Failed {
            return Err(CleanStoreError::InvalidTransition);
        }
        let id = action.target_id.ok_or(CleanStoreError::CorruptState)?;
        let mut change_request = self.change_request(&id)?;
        if change_request.resource_version != expected_version {
            return Err(CleanStoreError::StaleVersion);
        }
        if change_request.state != v1::ChangeRequestState::Failed as i32 {
            return Err(CleanStoreError::InvalidTransition);
        }
        let previous_version = change_request.resource_version;
        change_request.state = v1::ChangeRequestState::Publishing as i32;
        change_request.failure = None;
        change_request.updated_at = Some(timestamp(unix_seconds()?));
        change_request.resource_version = previous_version.saturating_add(1);
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        persist_change_request_update(&transaction, &change_request, previous_version)?;
        let changed = transaction
            .execute(
                "UPDATE durable_actions SET state='pending',failure=NULL,updated_at=?1
                 WHERE operation='PublishCandidate' AND idempotency_key=?2 AND state='failed'",
                params![unix_seconds()?, operation_key],
            )
            .map_err(CleanStoreError::Database)?;
        if changed != 1 {
            return Err(CleanStoreError::InvalidTransition);
        }
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(change_request)
    }

    pub(crate) fn list_change_requests(
        &mut self,
        repository_connection_id: Option<&str>,
        candidate_id: Option<&str>,
        state: Option<i32>,
        page: Option<&v1::PageRequest>,
    ) -> Result<Page<v1::ChangeRequest>, CleanStoreError> {
        let state_text = state.map_or_else(String::new, |value| value.to_string());
        let filter = encode_filter(&[
            repository_connection_id.unwrap_or(""),
            candidate_id.unwrap_or(""),
            &state_text,
        ]);
        self.page_messages(
            "ListChangeRequests",
            &filter,
            page,
            |connection| {
                let mut statement = connection
                    .prepare(
                        "SELECT payload FROM change_requests
                         WHERE (?1 IS NULL OR repository_connection_id=?1)
                           AND (?2 IS NULL OR candidate_id=?2)
                           AND (?3 IS NULL OR state=?3)
                         ORDER BY id",
                    )
                    .map_err(CleanStoreError::Database)?;
                let mut rows = statement
                    .query(params![repository_connection_id, candidate_id, state])
                    .map_err(CleanStoreError::Database)?;
                collect_bounded_payloads(&mut rows)
            },
            REVIEW_SERVICE,
        )
    }

    pub(crate) fn record_assessment(
        &mut self,
        mut assessment: v1::Assessment,
    ) -> Result<v1::Assessment, CleanStoreError> {
        if !valid_assessment_report(&assessment) {
            return Err(CleanStoreError::BoundExceeded);
        }
        let change_request = self.change_request(&assessment.change_request_id)?;
        if assessment.repository_connection_id != change_request.repository_connection_id {
            return Err(CleanStoreError::ParentMismatch);
        }
        if assessment.id.is_empty() {
            assessment.id = resource_id("assessment");
        }
        assessment.resource_version = 1;
        if assessment.created_at.is_none() {
            assessment.created_at = Some(timestamp(unix_seconds()?));
        }
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let cursor = append_event(
            &transaction,
            REVIEW_SERVICE,
            "assessment",
            &assessment.id,
            1,
            Some(&assessment.repository_connection_id),
            Some(&assessment.change_request_id),
            &assessment,
        )?;
        transaction
            .execute(
                "INSERT INTO assessments(
                    id,repository_connection_id,change_request_id,head_revision,conclusion,
                    payload,resource_version,event_cursor
                 ) VALUES (?1,?2,?3,?4,?5,?6,1,?7)",
                params![
                    assessment.id,
                    assessment.repository_connection_id,
                    assessment.change_request_id,
                    assessment.head_revision,
                    assessment.conclusion,
                    assessment.encode_to_vec(),
                    i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(map_constraint)?;
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(assessment)
    }

    pub(crate) fn assessment(&self, id: &str) -> Result<v1::Assessment, CleanStoreError> {
        get_payload(
            &self.connection,
            "SELECT payload FROM assessments WHERE id=?1",
            id,
            "assessment",
        )
    }

    pub(crate) fn list_assessments(
        &mut self,
        change_request_id: &str,
        head_revision: Option<&str>,
        conclusion: Option<i32>,
        page: Option<&v1::PageRequest>,
    ) -> Result<Page<v1::Assessment>, CleanStoreError> {
        self.change_request(change_request_id)?;
        let conclusion_text = conclusion.map_or_else(String::new, |value| value.to_string());
        let filter = encode_filter(&[
            change_request_id,
            head_revision.unwrap_or(""),
            &conclusion_text,
        ]);
        self.page_messages(
            "ListAssessments",
            &filter,
            page,
            |connection| {
                let mut statement = connection
                    .prepare(
                        "SELECT payload FROM assessments
                         WHERE change_request_id=?1
                           AND (?2 IS NULL OR head_revision=?2)
                           AND (?3 IS NULL OR conclusion=?3)
                         ORDER BY id",
                    )
                    .map_err(CleanStoreError::Database)?;
                let mut rows = statement
                    .query(params![change_request_id, head_revision, conclusion])
                    .map_err(CleanStoreError::Database)?;
                collect_bounded_payloads(&mut rows)
            },
            REVIEW_SERVICE,
        )
    }

    pub(crate) fn record_evidence_reference(
        &mut self,
        mut evidence: v1::EvidenceReference,
    ) -> Result<v1::EvidenceReference, CleanStoreError> {
        if !valid_evidence_report(&evidence) {
            return Err(CleanStoreError::BoundExceeded);
        }
        let assessment = self.assessment(&evidence.assessment_id)?;
        if evidence.id.is_empty() {
            evidence.id = resource_id("evidence");
        }
        evidence.resource_version = 1;
        let transaction = self
            .connection
            .transaction()
            .map_err(CleanStoreError::Database)?;
        let cursor = append_event(
            &transaction,
            REVIEW_SERVICE,
            "evidence_reference",
            &evidence.id,
            1,
            Some(&assessment.repository_connection_id),
            Some(&assessment.change_request_id),
            &evidence,
        )?;
        transaction
            .execute(
                "INSERT INTO evidence_references(
                    id,assessment_id,payload,resource_version,event_cursor
                 ) VALUES (?1,?2,?3,1,?4)",
                params![
                    evidence.id,
                    evidence.assessment_id,
                    evidence.encode_to_vec(),
                    i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?
                ],
            )
            .map_err(map_constraint)?;
        transaction.commit().map_err(CleanStoreError::Database)?;
        Ok(evidence)
    }

    pub(crate) fn evidence_reference(
        &self,
        id: &str,
    ) -> Result<v1::EvidenceReference, CleanStoreError> {
        get_payload(
            &self.connection,
            "SELECT payload FROM evidence_references WHERE id=?1",
            id,
            "evidence_reference",
        )
    }

    pub(crate) fn list_evidence_references(
        &mut self,
        assessment_id: &str,
        page: Option<&v1::PageRequest>,
    ) -> Result<Page<v1::EvidenceReference>, CleanStoreError> {
        self.assessment(assessment_id)?;
        let filter = encode_filter(&[assessment_id]);
        self.page_messages(
            "ListEvidenceReferences",
            &filter,
            page,
            |connection| {
                query_payloads(
                    connection,
                    "evidence_references",
                    "assessment_id",
                    None,
                    Some(assessment_id),
                )
            },
            REVIEW_SERVICE,
        )
    }
}
