//! Immutable research versions and atomic approval of the reviewed task set.
use super::{CleanStore, CleanStoreError, Connection, Message, decode, params, v1};

pub(super) fn migrate(connection: &Connection) -> Result<(), CleanStoreError> {
    connection
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(CleanStoreError::Database)?;
    let result = (|| {
        let payloads = connection
            .prepare("SELECT payload,id,project_id,cursor FROM project_conversations")
            .map_err(CleanStoreError::Database)?
            .query_map([], |r| {
                Ok((
                    r.get::<_, Vec<u8>>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })
            .map_err(CleanStoreError::Database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(CleanStoreError::Database)?;
        for (payload, id, project, cursor) in payloads {
            let mut c: v1::ProjectConversation = decode(&payload)?;
            super::project_conversations::validate_conversation_index(
                &c,
                &id,
                &project,
                cursor,
                payload.len(),
            )?;
            super::project_conversations::validate_conversation_runs(&c)?;
            if c.plan.is_some() {
                return Err(CleanStoreError::CorruptState);
            }
            let run_ids = c.runs.iter().map(|r| r.id.clone()).collect::<Vec<_>>();
            for run_id in run_ids {
                append_version(&mut c, 0, &run_id, &[])?;
            }
            validate(&c)?;
            if c.plan.is_some() {
                c.update_cursor = c
                    .update_cursor
                    .checked_add(1)
                    .ok_or(CleanStoreError::BoundExceeded)?;
            }
            if c.encoded_len() > 3 * 1024 * 1024 {
                return Err(CleanStoreError::BoundExceeded);
            }
            connection
                .execute(
                    "UPDATE project_conversations SET payload=?1,cursor=?3 WHERE id=?2",
                    params![
                        c.encode_to_vec(),
                        c.id,
                        i64::try_from(c.update_cursor)
                            .map_err(|_| CleanStoreError::BoundExceeded)?
                    ],
                )
                .map_err(CleanStoreError::Database)?;
        }
        connection
            .execute_batch("UPDATE clean_schema SET version=21 WHERE component='control_plane'")
            .map_err(CleanStoreError::Database)
    })();
    finish(connection, result)
}

pub(super) fn append_version(
    c: &mut v1::ProjectConversation,
    first: usize,
    run_id: &str,
    metadata: &[(String, String, String)],
) -> Result<(), CleanStoreError> {
    let proposals = c.proposals[first..]
        .iter()
        .filter(|p| p.action == "create_task" && p.run_id == run_id)
        .collect::<Vec<_>>();
    if proposals.is_empty() {
        return Ok(());
    }
    let plan = c.plan.get_or_insert_with(|| v1::ProjectPlan {
        id: format!("plan-{}", c.id),
        versions: vec![],
    });
    if plan.versions.len() >= 512 {
        return Err(CleanStoreError::BoundExceeded);
    }
    if let Some(prior) = plan.versions.last_mut()
        && prior.state == "pending"
    {
        prior.state = "superseded".into();
    }
    let state = if proposals.iter().all(|p| p.state == "approved") {
        "approved"
    } else if proposals.iter().all(|p| p.state == "rejected") {
        "rejected"
    } else if proposals.iter().all(|p| p.state == "revise") {
        "revise"
    } else if proposals.iter().all(|p| p.state == "pending") {
        "pending"
    } else {
        "superseded"
    };
    let metadata_text = |field: usize| {
        metadata
            .iter()
            .map(|m| match field {
                0 => m.0.as_str(),
                1 => m.1.as_str(),
                _ => m.2.as_str(),
            })
            .filter(|s| !s.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    };
    plan.versions.push(v1::ProjectPlanVersion {
        version: u64::try_from(plan.versions.len() + 1)
            .map_err(|_| CleanStoreError::BoundExceeded)?,
        run_id: run_id.into(),
        state: state.into(),
        findings: metadata_text(0),
        scope: metadata_text(1),
        dependency_environment_impact: metadata_text(2),
        acceptance_criteria: proposals
            .iter()
            .flat_map(|p| p.request.iter().flat_map(|r| r.acceptance_criteria.clone()))
            .collect(),
        proposal_ids: proposals.iter().map(|p| p.id.clone()).collect(),
        created_at: c
            .runs
            .iter()
            .find(|r| r.id == run_id)
            .and_then(|r| r.created_at),
        failure: String::new(),
    });
    Ok(())
}

pub(super) fn validate(c: &v1::ProjectConversation) -> Result<(), CleanStoreError> {
    let Some(plan) = &c.plan else {
        if c.proposals.iter().any(|p| p.action == "create_task") {
            return Err(CleanStoreError::CorruptState);
        }
        return Ok(());
    };
    if plan.id.is_empty() || plan.versions.is_empty() || plan.versions.len() > 512 {
        return Err(CleanStoreError::CorruptState);
    }
    let mut ids = std::collections::BTreeSet::new();
    for (index, version) in plan.versions.iter().enumerate() {
        if version.version != (index + 1) as u64
            || version.proposal_ids.is_empty()
            || !matches!(
                version.state.as_str(),
                "pending" | "approved" | "rejected" | "revise" | "superseded"
            )
            || (version.state == "pending" && index + 1 != plan.versions.len())
            || version.findings.len() > 100 * (16_384 + 2)
            || version.scope.len() > 100 * (16_384 + 2)
            || version.dependency_environment_impact.len() > 100 * (16_384 + 2)
        {
            return Err(CleanStoreError::CorruptState);
        }
        let expected_criteria = version
            .proposal_ids
            .iter()
            .filter_map(|id| c.proposals.iter().find(|p| &p.id == id))
            .flat_map(|p| p.request.iter().flat_map(|r| r.acceptance_criteria.clone()))
            .collect::<Vec<_>>();
        if expected_criteria != version.acceptance_criteria {
            return Err(CleanStoreError::CorruptState);
        }
        for id in &version.proposal_ids {
            let p = c
                .proposals
                .iter()
                .find(|p| &p.id == id)
                .ok_or(CleanStoreError::CorruptState)?;
            if !ids.insert(id)
                || p.run_id != version.run_id
                || p.action != "create_task"
                || (version.state == "approved" && p.state != "approved")
            {
                return Err(CleanStoreError::CorruptState);
            }
        }
    }
    if c.proposals
        .iter()
        .filter(|p| p.action == "create_task")
        .any(|p| !ids.contains(&p.id))
    {
        return Err(CleanStoreError::CorruptState);
    }
    Ok(())
}
impl CleanStore {
    pub(crate) fn decide_project_plan(
        &mut self,
        key: &str,
        message: &v1::DecideProjectPlanRequest,
    ) -> Result<v1::ProjectConversation, CleanStoreError> {
        self.decide_plan_receipt(key, message, message)
    }
    fn decide_plan_receipt<M: Message>(
        &mut self,
        key: &str,
        receipt: &M,
        message: &v1::DecideProjectPlanRequest,
    ) -> Result<v1::ProjectConversation, CleanStoreError> {
        self.connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(CleanStoreError::Database)?;
        let result = (|| {
            if let Some(c) = self.replay_conversation(key, receipt)? {
                return Ok(c);
            }
            let mut c = self.project_conversation(&message.conversation_id)?;
            let plan = c.plan.as_ref().ok_or(CleanStoreError::InvalidTransition)?;
            if plan.id != message.plan_id {
                return Err(CleanStoreError::ParentMismatch);
            }
            let index = plan
                .versions
                .iter()
                .position(|v| v.version == message.version)
                .ok_or(CleanStoreError::StaleVersion)?;
            let version = &plan.versions[index];
            if version.state == "approved" && message.decision == "approve" {
                self.remember_conversation(key, receipt, &c.id)?;
                return Ok(c);
            }
            if index + 1 != plan.versions.len() {
                return Err(CleanStoreError::StaleVersion);
            }
            if version.state != "pending" {
                return Err(CleanStoreError::InvalidTransition);
            }
            if c.runs
                .iter()
                .any(|r| matches!(r.state.as_str(), "queued" | "running") && r.purpose != "summary")
            {
                return Err(CleanStoreError::StaleVersion);
            }
            let proposal_ids = version.proposal_ids.clone();
            let plan_id = message.plan_id.clone();
            let research_run_id = version.run_id.clone();
            let state = match message.decision.as_str() {
                "approve" => "approved",
                "reject" => "rejected",
                "revise" => "revise",
                _ => return Err(CleanStoreError::InvalidTransition),
            };
            for id in proposal_ids {
                let p = c
                    .proposals
                    .iter_mut()
                    .find(|p| p.id == id)
                    .ok_or(CleanStoreError::CorruptState)?;
                // Historical individual approvals remain admitted and immutable.
                if p.state == "approved" {
                    continue;
                }
                if state == "approved" {
                    let (task, attempt) = self.approve_project_proposal(
                        &c.project_id,
                        p,
                        Some(v1::TaskOrigin {
                            project_id: c.project_id.clone(),
                            conversation_id: c.id.clone(),
                            plan_id: plan_id.clone(),
                            plan_version: message.version,
                            proposal_id: p.id.clone(),
                            research_run_id: research_run_id.clone(),
                        }),
                    )?;
                    p.task_id = task;
                    p.attempt_id = attempt;
                }
                p.state = state.into();
            }
            c.plan
                .as_mut()
                .ok_or(CleanStoreError::CorruptState)?
                .versions[index]
                .state = state.into();
            validate(&c)?;
            self.save_conversation(&mut c)?;
            self.remember_conversation(key, receipt, &c.id)?;
            Ok(c)
        })();
        match result {
            Ok(c) => {
                finish(&self.connection, Ok(()))?;
                Ok(c)
            }
            Err(e) => {
                let _ = self.connection.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }
}
fn finish(
    connection: &Connection,
    result: Result<(), CleanStoreError>,
) -> Result<(), CleanStoreError> {
    match result {
        Ok(()) => connection
            .execute_batch("COMMIT")
            .map_err(CleanStoreError::Database),
        Err(e) => {
            let _ = connection.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}
