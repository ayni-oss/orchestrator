//! Admit summary turns using current project defaults and fresh credentials at dispatch.
use super::project_conversations::finish_transaction;
use super::{
    CleanStore, CleanStoreError, Digest, OptionalExtension, Sha256, decode, is_terminal_attempt,
    params, resource_id, timestamp, unix_seconds, v1,
};
impl CleanStore {
    pub(crate) fn queue_project_outcome_summaries(&mut self) -> Result<(), CleanStoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT payload FROM project_conversations ORDER BY id LIMIT 1000")
            .map_err(CleanStoreError::Database)?;
        let payloads = statement
            .query_map([], |r| r.get::<_, Vec<u8>>(0))
            .map_err(CleanStoreError::Database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(CleanStoreError::Database)?;
        drop(statement);
        for payload in payloads {
            let mut c: v1::ProjectConversation = decode(&payload)?;
            self.conversation_session_projection(&mut c)?;
            if c.runs
                .iter()
                .any(|r| matches!(r.state.as_str(), "queued" | "running"))
            {
                continue;
            }
            let proposals = c.proposals.clone();
            for proposal in proposals
                .into_iter()
                .filter(|p| p.state == "approved" && !p.attempt_id.is_empty())
            {
                let attempt = self.development_attempt(&proposal.attempt_id)?;
                if !is_terminal_attempt(attempt.state)
                    && attempt.state != v1::DevelopmentAttemptState::WaitingForInput as i32
                {
                    continue;
                }
                let outcome = format!(
                    "{}:{:x}",
                    attempt.state,
                    Sha256::digest(
                        format!("{:?}:{:?}", attempt.result, attempt.requested_input).as_bytes()
                    )
                );
                let exists=self.connection.query_row("SELECT 1 FROM project_outcome_summaries WHERE attempt_id=?1 AND outcome=?2",params![attempt.id,outcome],|r|r.get::<_,i32>(0)).optional().map_err(CleanStoreError::Database)?.is_some();
                if exists {
                    continue;
                }
                let project = self.project(&c.project_id)?;
                let Some(defaults) = self.effective_project_defaults(&project)? else {
                    continue;
                };
                let run = v1::ProjectAgentRun {
                    id: resource_id("run"),
                    purpose: "summary".into(),
                    state: "queued".into(),
                    model: defaults.project_model,
                    connection_id: defaults.project_connection_id,
                    session_managed: true,
                    created_at: Some(timestamp(unix_seconds()?)),
                    ..Default::default()
                };
                let safe_outcome =
                    super::project_context::safe_outcome(&proposal.task_id, &attempt);
                let request = ayni_orchestrator_runtime::project_agent::ProjectAgentRequest {
                    run_id: run.id.clone(),
                    project_id: c.project_id.clone(),
                    model: run.model.clone(),
                    history: String::new(),
                    conversation_context: serde_json::Value::Null,
                    repositories: Vec::new(),
                    context: crate::truncate_utf8(
                        &format!(
                            "Summarize the accepted outcome. Explicitly distinguish execution, quality and publication. Never propose or initiate actions. {safe_outcome}"
                        ),
                        65536,
                    ),
                    max_model_calls: 1,
                    deadline_seconds: 600,
                };
                self.connection
                    .execute_batch("BEGIN IMMEDIATE")
                    .map_err(CleanStoreError::Database)?;
                let result = (|| {
                    self.save_project_run_request(&run.id, &request)?;
                    self.connection.execute("INSERT INTO project_outcome_summaries(attempt_id,outcome,conversation_id) VALUES(?1,?2,?3)",params![attempt.id,outcome,c.id]).map_err(CleanStoreError::Database)?;
                    super::project_context::archive_outcome(
                        &self.connection,
                        &c,
                        &attempt,
                        &safe_outcome,
                    )?;
                    c.runs.push(run);
                    self.note_planner_message(&c, unix_seconds()?)?;
                    self.save_conversation(&mut c)
                })();
                finish_transaction(&self.connection, result)?;
                break;
            }
        }
        Ok(())
    }
}
