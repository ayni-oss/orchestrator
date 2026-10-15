//! Durable project conversations, approval receipts and serialized research runs.
use super::{
    CleanStore, CleanStoreError, Connection, Digest, Message, OptionalExtension, Sha256, decode,
    get_payload, is_terminal_attempt, params, resource_id, timestamp, unix_seconds, v1,
};
use ayni_orchestrator_runtime::project_agent::{
    ProjectAgentRequest, ProjectAgentResult, ResearchProposal,
};

pub(crate) const SCHEMA: &str = "CREATE TABLE project_agent_requests(run_id TEXT PRIMARY KEY,payload TEXT NOT NULL); CREATE TABLE project_conversations(id TEXT PRIMARY KEY,project_id TEXT NOT NULL REFERENCES projects(id),cursor INTEGER NOT NULL,payload BLOB NOT NULL); CREATE TABLE project_conversation_actions(action_key TEXT PRIMARY KEY,digest BLOB NOT NULL,conversation_id TEXT NOT NULL REFERENCES project_conversations(id)); CREATE TABLE project_outcome_summaries(attempt_id TEXT NOT NULL,outcome TEXT NOT NULL,conversation_id TEXT NOT NULL,PRIMARY KEY(attempt_id,outcome));";

pub(super) fn migrate(connection: &Connection) -> Result<(), CleanStoreError> {
    connection.execute_batch(&format!("BEGIN IMMEDIATE; {SCHEMA} UPDATE clean_schema SET version=19 WHERE component='control_plane'; COMMIT;")).map_err(CleanStoreError::Database)
}
impl CleanStore {
    pub(super) fn validate_project_conversations(&self) -> Result<(), CleanStoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT id,project_id,cursor,payload FROM project_conversations")
            .map_err(CleanStoreError::Database)?;
        let mut rows = statement.query([]).map_err(CleanStoreError::Database)?;
        while let Some(row) = rows.next().map_err(CleanStoreError::Database)? {
            let id: String = row.get(0).map_err(CleanStoreError::Database)?;
            let project: String = row.get(1).map_err(CleanStoreError::Database)?;
            let cursor: i64 = row.get(2).map_err(CleanStoreError::Database)?;
            let payload: Vec<u8> = row.get(3).map_err(CleanStoreError::Database)?;
            let conversation: v1::ProjectConversation = decode(&payload)?;
            validate_conversation_index(&conversation, &id, &project, cursor, payload.len())?;
            validate_conversation_runs(&conversation)?;
            super::project_plans::validate(&conversation)?;
        }
        self.validate_project_agent_requests()
    }
    fn validate_project_agent_requests(&self) -> Result<(), CleanStoreError> {
        let mut requests = self
            .connection
            .prepare("SELECT run_id,payload FROM project_agent_requests")
            .map_err(CleanStoreError::Database)?;
        let mut rows = requests.query([]).map_err(CleanStoreError::Database)?;
        while let Some(row) = rows.next().map_err(CleanStoreError::Database)? {
            let id: String = row.get(0).map_err(CleanStoreError::Database)?;
            let payload: String = row.get(1).map_err(CleanStoreError::Database)?;
            let request: ProjectAgentRequest =
                serde_json::from_str(&payload).map_err(|_| CleanStoreError::CorruptState)?;
            if id != request.run_id || payload.len() > 512 * 1024 {
                return Err(CleanStoreError::CorruptState);
            }
        }
        Ok(())
    }
    pub(crate) fn project_run_request(
        &self,
        id: &str,
    ) -> Result<
        Option<ayni_orchestrator_runtime::project_agent::ProjectAgentRequest>,
        CleanStoreError,
    > {
        let payload = self
            .connection
            .query_row(
                "SELECT payload FROM project_agent_requests WHERE run_id=?1",
                [id],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(CleanStoreError::Database)?;
        payload
            .map(|p| serde_json::from_str(&p).map_err(|_| CleanStoreError::CorruptState))
            .transpose()
    }
    pub(crate) fn save_project_run_request(
        &mut self,
        id: &str,
        request: &ayni_orchestrator_runtime::project_agent::ProjectAgentRequest,
    ) -> Result<(), CleanStoreError> {
        let payload = serde_json::to_string(request).map_err(|_| CleanStoreError::CorruptState)?;
        if payload.len() > 512 * 1024 {
            return Err(CleanStoreError::BoundExceeded);
        }
        if let Some(existing) = self.project_run_request(id)? {
            if serde_json::to_string(&existing).map_err(|_| CleanStoreError::CorruptState)?
                != payload
            {
                return Err(CleanStoreError::StaleVersion);
            }
            return Ok(());
        }
        self.connection
            .execute(
                "INSERT INTO project_agent_requests(run_id,payload) VALUES(?1,?2)",
                params![id, payload],
            )
            .map_err(CleanStoreError::Database)?;
        Ok(())
    }

    pub(crate) fn project_conversation(
        &self,
        id: &str,
    ) -> Result<v1::ProjectConversation, CleanStoreError> {
        get_payload(
            &self.connection,
            "SELECT payload FROM project_conversations WHERE id=?1",
            id,
            "project_conversation",
        )
    }
    pub(crate) fn list_project_conversations(
        &self,
        project: &str,
    ) -> Result<Vec<v1::ProjectConversation>, CleanStoreError> {
        self.project(project)?;
        let mut statement=self.connection.prepare("SELECT payload FROM project_conversations WHERE project_id=?1 ORDER BY id DESC LIMIT 100").map_err(CleanStoreError::Database)?;
        let mut rows = statement
            .query([project])
            .map_err(CleanStoreError::Database)?;
        let mut conversations = Vec::new();
        while let Some(row) = rows.next().map_err(CleanStoreError::Database)? {
            let payload: Vec<u8> = row.get(0).map_err(CleanStoreError::Database)?;
            let mut conversation: v1::ProjectConversation = decode(&payload)?;
            conversation.messages.clear();
            conversation.runs.clear();
            conversation.proposals.clear();
            conversation.plan = None;
            conversations.push(conversation);
        }
        Ok(conversations)
    }
    pub(crate) fn replay_conversation<M: Message>(
        &self,
        key: &str,
        request: &M,
    ) -> Result<Option<v1::ProjectConversation>, CleanStoreError> {
        let receipt=self.connection.query_row("SELECT digest,conversation_id FROM project_conversation_actions WHERE action_key=?1",[key],|r|Ok((r.get::<_,Vec<u8>>(0)?,r.get::<_,String>(1)?))).optional().map_err(CleanStoreError::Database)?;
        receipt
            .map(|(digest, id)| {
                if digest != Sha256::digest(request.encode_to_vec()).to_vec() {
                    return Err(CleanStoreError::IdempotencyConflict);
                }
                self.project_conversation(&id)
            })
            .transpose()
    }
    pub(super) fn save_conversation(
        &mut self,
        conversation: &mut v1::ProjectConversation,
    ) -> Result<(), CleanStoreError> {
        if conversation.messages.len() > 512 || conversation.encode_to_vec().len() > 3 * 1024 * 1024
        {
            return Err(CleanStoreError::BoundExceeded);
        }
        conversation.update_cursor = conversation
            .update_cursor
            .checked_add(1)
            .ok_or(CleanStoreError::BoundExceeded)?;
        self.connection.execute("INSERT INTO project_conversations(id,project_id,cursor,payload) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET cursor=excluded.cursor,payload=excluded.payload",params![conversation.id,conversation.project_id,i64::try_from(conversation.update_cursor).map_err(|_|CleanStoreError::BoundExceeded)?,conversation.encode_to_vec()]).map_err(CleanStoreError::Database)?;
        Ok(())
    }
    pub(super) fn remember_conversation<M: Message>(
        &self,
        key: &str,
        request: &M,
        id: &str,
    ) -> Result<(), CleanStoreError> {
        self.connection.execute("INSERT INTO project_conversation_actions(action_key,digest,conversation_id) VALUES(?1,?2,?3)",params![key,Sha256::digest(request.encode_to_vec()).to_vec(),id]).map_err(CleanStoreError::Database)?;
        Ok(())
    }
    pub(crate) fn create_project_conversation(
        &mut self,
        key: &str,
        request: &v1::CreateProjectConversationRequest,
    ) -> Result<v1::ProjectConversation, CleanStoreError> {
        if let Some(c) = self.replay_conversation(key, request)? {
            return Ok(c);
        }
        if request.title.trim().is_empty() || request.title.len() > 256 {
            return Err(CleanStoreError::BoundExceeded);
        }
        let project = self.project(&request.project_id)?;
        let repositories = if request.repository_connection_ids.is_empty() {
            project.repository_connection_ids
        } else {
            request.repository_connection_ids.clone()
        };
        if repositories.is_empty() || repositories.len() > 100 {
            return Err(CleanStoreError::BoundExceeded);
        }
        let mut unique = std::collections::BTreeSet::new();
        for id in &repositories {
            self.project_member(&request.project_id, id)?;
            if !unique.insert(id) {
                return Err(CleanStoreError::ParentMismatch);
            }
        }
        let mut c = v1::ProjectConversation {
            id: resource_id("conversation"),
            project_id: request.project_id.clone(),
            title: request.title.clone(),
            repository_connection_ids: repositories,
            created_at: Some(timestamp(unix_seconds()?)),
            ..Default::default()
        };
        self.connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(CleanStoreError::Database)?;
        let result = (|| {
            self.save_conversation(&mut c)?;
            self.remember_conversation(key, request, &c.id)
        })();
        finish_transaction(&self.connection, result)?;
        Ok(c)
    }
    pub(crate) fn send_project_message(
        &mut self,
        key: &str,
        request: &v1::SendProjectMessageRequest,
        run: v1::ProjectAgentRun,
    ) -> Result<v1::ProjectConversation, CleanStoreError> {
        if let Some(c) = self.replay_conversation(key, request)? {
            return Ok(c);
        }
        if request.content.trim().is_empty() || request.content.len() > 16384 {
            return Err(CleanStoreError::BoundExceeded);
        }
        let mut c = self.project_conversation(&request.conversation_id)?;
        for id in &c.repository_connection_ids {
            self.project_member(&c.project_id, id)?;
        }
        if c.runs
            .iter()
            .any(|r| matches!(r.state.as_str(), "queued" | "running"))
        {
            return Err(CleanStoreError::InvalidTransition);
        }
        c.messages.push(v1::ProjectMessage {
            id: resource_id("message"),
            role: "user".into(),
            content: request.content.clone(),
            created_at: Some(timestamp(unix_seconds()?)),
            run_id: run.id.clone(),
        });
        c.runs.push(run);
        self.connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(CleanStoreError::Database)?;
        let result = (|| {
            self.save_conversation(&mut c)?;
            self.remember_conversation(key, request, &c.id)
        })();
        finish_transaction(&self.connection, result)?;
        Ok(c)
    }
    pub(crate) fn terminal_project_runs(
        &self,
    ) -> Result<Vec<(v1::ProjectConversation, v1::ProjectAgentRun)>, CleanStoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT payload FROM project_conversations ORDER BY id LIMIT 1000")
            .map_err(CleanStoreError::Database)?;
        let payloads = statement
            .query_map([], |r| r.get::<_, Vec<u8>>(0))
            .map_err(CleanStoreError::Database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(CleanStoreError::Database)?;
        let mut runs = Vec::new();
        for payload in payloads {
            let c: v1::ProjectConversation = decode(&payload)?;
            for r in &c.runs {
                if !matches!(r.state.as_str(), "queued" | "running") {
                    runs.push((c.clone(), r.clone()));
                }
            }
        }
        Ok(runs)
    }
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
                let original = c
                    .runs
                    .iter()
                    .find(|r| r.id == proposal.run_id)
                    .ok_or(CleanStoreError::CorruptState)?
                    .clone();
                let mut run = original;
                run.id = resource_id("run");
                run.purpose = "summary".into();
                run.state = "queued".into();
                run.model_calls = 0;
                run.failure.clear();
                run.created_at = Some(timestamp(unix_seconds()?));
                let task = self.development_task(&proposal.task_id)?;
                let request = ayni_orchestrator_runtime::project_agent::ProjectAgentRequest {
                    run_id: run.id.clone(),
                    project_id: c.project_id.clone(),
                    model: run.model.clone(),
                    history: String::new(),
                    repositories: Vec::new(),
                    context: crate::truncate_utf8(
                        &format!(
                            "Summarize the accepted outcome. Explicitly distinguish execution, quality and publication. Never propose or initiate actions. Task: {task:?} Attempt: {attempt:?}"
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
                    c.runs.push(run);
                    self.save_conversation(&mut c)
                })();
                finish_transaction(&self.connection, result)?;
                break;
            }
        }
        Ok(())
    }
    pub(crate) fn queued_project_runs(
        &self,
    ) -> Result<Vec<(v1::ProjectConversation, v1::ProjectAgentRun)>, CleanStoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT payload FROM project_conversations ORDER BY id LIMIT 1000")
            .map_err(CleanStoreError::Database)?;
        let payloads = statement
            .query_map([], |r| r.get::<_, Vec<u8>>(0))
            .map_err(CleanStoreError::Database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(CleanStoreError::Database)?;
        let mut runs = Vec::new();
        for payload in payloads {
            let c: v1::ProjectConversation = decode(&payload)?;
            for r in &c.runs {
                if matches!(r.state.as_str(), "queued" | "running") {
                    runs.push((c.clone(), r.clone()));
                }
            }
        }
        Ok(runs)
    }
    pub(crate) fn mark_project_run_running(&mut self, id: &str) -> Result<(), CleanStoreError> {
        self.update_project_run(id, "running", "")
    }
    pub(crate) fn fail_project_run(
        &mut self,
        id: &str,
        reason: &str,
    ) -> Result<(), CleanStoreError> {
        self.update_project_run(id, "interrupted", reason)
    }
    fn update_project_run(
        &mut self,
        id: &str,
        state: &str,
        reason: &str,
    ) -> Result<(), CleanStoreError> {
        let Some((mut c, _)) = self
            .queued_project_runs()?
            .into_iter()
            .find(|(_, r)| r.id == id)
        else {
            return Ok(());
        };
        let run = c
            .runs
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or(CleanStoreError::CorruptState)?;
        run.state = state.into();
        run.failure = crate::truncate_utf8(reason, 512);
        self.save_conversation(&mut c)
    }
    pub(crate) fn complete_project_run(
        &mut self,
        result: ProjectAgentResult,
    ) -> Result<(), CleanStoreError> {
        let Some((mut c, _)) = self
            .queued_project_runs()?
            .into_iter()
            .find(|(_, r)| r.id == result.run_id)
        else {
            return Ok(());
        };
        let researched = self.project_run_request(&result.run_id)?;
        validate_project_result(&c, &result, researched.as_ref())?;
        let mut targets = std::collections::BTreeSet::new();
        let first_new = c.proposals.len();
        let metadata = result
            .proposals
            .iter()
            .filter(|p| p.action == "create_task")
            .map(|p| {
                (
                    p.findings.clone(),
                    p.scope.clone(),
                    p.dependency_environment_impact.clone(),
                )
            })
            .collect::<Vec<_>>();
        for proposal in result.proposals {
            if proposal.findings.len() > 16384
                || proposal.scope.len() > 16384
                || proposal.dependency_environment_impact.len() > 16384
            {
                return Err(CleanStoreError::BoundExceeded);
            }
            self.validate_research_proposal(&c, &proposal, researched.as_ref(), &mut targets)?;
            let request = proposal_request(&c.project_id, &proposal)?;
            let researched_runtime = if proposal.action == "create_task" {
                api_validation_task(&request)?;
                Some(self.researched_proposal_runtime(&result.run_id, &request)?)
            } else {
                None
            };
            c.proposals.push(v1::ProjectTaskProposal {
                researched_runtime,
                id: resource_id("proposal"),
                run_id: result.run_id.clone(),
                state: "pending".into(),
                request: Some(request),
                action: proposal.action,
                target_task_id: proposal.target_task_id,
                ..Default::default()
            });
        }
        super::project_plans::append_version(&mut c, first_new, &result.run_id, &metadata)?;
        super::project_plans::validate(&c)?;
        let run = c
            .runs
            .iter_mut()
            .find(|r| r.id == result.run_id)
            .ok_or(CleanStoreError::CorruptState)?;
        run.state = "completed".into();
        run.model_calls = result.model_calls;
        run.token_usage = Some(v1::ProviderTokenUsage {
            input_tokens: result.token_usage.input_tokens,
            output_tokens: result.token_usage.output_tokens,
            cached_input_tokens: result.token_usage.cached_input_tokens,
            reasoning_tokens: result.token_usage.reasoning_tokens,
            total_tokens: result.token_usage.total_tokens,
        });
        c.messages.push(v1::ProjectMessage {
            id: resource_id("message"),
            role: "assistant".into(),
            content: result.message,
            run_id: result.run_id,
            created_at: Some(timestamp(unix_seconds()?)),
        });
        self.save_conversation(&mut c)
    }
    fn validate_research_proposal(
        &self,
        c: &v1::ProjectConversation,
        p: &ResearchProposal,
        researched: Option<&ProjectAgentRequest>,
        targets: &mut std::collections::BTreeSet<String>,
    ) -> Result<(), CleanStoreError> {
        if researched.is_none_or(|r| {
            !r.repositories.iter().any(|repository| {
                let same_repository = repository.id == p.repository_id;
                let same_revision = repository.revision == p.revision;
                same_repository && same_revision
            })
        }) {
            return Err(CleanStoreError::StaleVersion);
        }
        if !c.repository_connection_ids.contains(&p.repository_id)
            || !targets.insert(p.repository_id.clone())
        {
            return Err(CleanStoreError::ParentMismatch);
        }
        self.project_member(&c.project_id, &p.repository_id)?;
        if !matches!(
            p.action.as_str(),
            "create_task" | "cancel_task" | "retry_task" | "retry_publication"
        ) {
            return Err(CleanStoreError::InvalidTransition);
        }
        if p.action != "create_task" {
            self.validate_proposed_lifecycle_action(c, p)?;
        }
        Ok(())
    }
    fn validate_proposed_lifecycle_action(
        &self,
        c: &v1::ProjectConversation,
        p: &ResearchProposal,
    ) -> Result<(), CleanStoreError> {
        let task = self.development_task(&p.target_task_id)?;
        if task.project_id != c.project_id {
            return Err(CleanStoreError::ParentMismatch);
        }
        let last = c
            .messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map_or("", |m| m.content.as_str())
            .to_ascii_lowercase();
        if !ayni_orchestrator_runtime::project_agent::explicit_action_request(
            &last,
            &p.action,
            &p.target_task_id,
        ) {
            return Err(CleanStoreError::InvalidTransition);
        }
        if task
            .snapshot
            .as_ref()
            .and_then(|s| s.repository.as_ref())
            .is_none_or(|r| r.repository_connection_id != p.repository_id)
        {
            return Err(CleanStoreError::ParentMismatch);
        }
        Ok(())
    }
    fn researched_proposal_runtime(
        &self,
        run_id: &str,
        request: &v1::DevelopmentTaskRequest,
    ) -> Result<v1::FrozenRuntime, CleanStoreError> {
        let frozen = self
            .project_run_request(run_id)?
            .ok_or(CleanStoreError::CorruptState)?;
        let context: serde_json::Value =
            serde_json::from_str(&frozen.context).map_err(|_| CleanStoreError::CorruptState)?;
        let profile = context["repositories"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|r| {
                r["repository_id"].as_str() == Some(request.repository_connection_id.as_str())
            })
            .flat_map(|r| r["profiles"].as_array().into_iter().flatten())
            .find(|r| r["id"].as_str() == Some(request.runtime_profile_id.as_str()))
            .ok_or(CleanStoreError::ProfileNotReady)?;
        Ok(v1::FrozenRuntime {
            runtime_profile_id: request.runtime_profile_id.clone(),
            runtime_profile_revision_id: profile["active_revision_id"]
                .as_str()
                .ok_or(CleanStoreError::ProfileNotReady)?
                .into(),
            runtime_artifact_id: profile["active_runtime_artifact_id"]
                .as_str()
                .ok_or(CleanStoreError::ProfileNotReady)?
                .into(),
            resolved_digest: profile["resolved_digest"]
                .as_str()
                .ok_or(CleanStoreError::ProfileNotReady)?
                .into(),
            ..Default::default()
        })
    }
    pub(crate) fn decide_project_proposal(
        &mut self,
        key: &str,
        message: &v1::DecideProjectProposalRequest,
    ) -> Result<v1::ProjectConversation, CleanStoreError> {
        if let Some(c) = self.replay_conversation(key, message)? {
            return Ok(c);
        }
        let mut c = self.project_conversation(&message.conversation_id)?;
        let index = c
            .proposals
            .iter()
            .position(|p| p.id == message.proposal_id)
            .ok_or(CleanStoreError::InvalidTransition)?;
        if c.proposals[index].action == "create_task" {
            if c.proposals[index].state == "approved" && message.decision == "approve" {
                self.remember_conversation(key, message, &c.id)?;
                return Ok(c);
            }
            return Err(CleanStoreError::InvalidTransition);
        }
        let p = &mut c.proposals[index];
        if p.state == "approved" && message.decision == "approve" {
            return Ok(c);
        }
        if p.state != "pending" {
            return Err(CleanStoreError::InvalidTransition);
        }
        match message.decision.as_str() {
            "reject" => p.state = "rejected".into(),
            "revise" => p.state = "revise".into(),
            "approve" => {
                let (task, attempt) = self.approve_project_proposal(&c.project_id, p)?;
                p.task_id = task;
                p.attempt_id = attempt;
                p.state = "approved".into();
            }
            _ => return Err(CleanStoreError::InvalidTransition),
        }
        self.connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(CleanStoreError::Database)?;
        let result = (|| {
            self.save_conversation(&mut c)?;
            self.remember_conversation(key, message, &c.id)
        })();
        finish_transaction(&self.connection, result)?;
        Ok(c)
    }
    pub(super) fn approve_project_proposal(
        &mut self,
        project_id: &str,
        p: &v1::ProjectTaskProposal,
    ) -> Result<(String, String), CleanStoreError> {
        let requested = p.request.as_ref().ok_or(CleanStoreError::CorruptState)?;
        self.project_member(project_id, &requested.repository_connection_id)?;
        let stable_key = format!("project-proposal-{}", p.id);
        let (task, attempt) = match p.action.as_str() {
            "create_task" => self.admit_project_proposal(&stable_key, p)?,
            "cancel_task" => {
                let task = self.development_task(&p.target_task_id)?;
                let cmd = crate::adapters::grpc::task_mapping::cancel_command(
                    &stable_key,
                    &v1::CancelDevelopmentTaskRequest {
                        idempotency_key: stable_key.clone(),
                        development_task_id: task.id.clone(),
                    },
                );
                let receipt = ayni_orchestrator_application::tasks::cancellation::cancel(self, cmd)
                    .map_err(super::task_cancellation::cancellation_error)?;
                (receipt, task.active_attempt_id.unwrap_or_default())
            }
            "retry_task" => {
                let cmd = crate::adapters::grpc::task_mapping::retry_command(
                    &stable_key,
                    &v1::RetryDevelopmentTaskRequest {
                        idempotency_key: stable_key.clone(),
                        development_task_id: p.target_task_id.clone(),
                    },
                );
                let receipt = ayni_orchestrator_application::tasks::retry::retry(self, &cmd)
                    .map_err(super::task_retry::retry_error)?;
                (p.target_task_id.clone(), receipt)
            }
            "retry_publication" => {
                let task = self.development_task(&p.target_task_id)?;
                let attempt = self.retry_task_publication(
                    &stable_key,
                    &v1::RetryTaskPublicationRequest {
                        idempotency_key: stable_key.clone(),
                        development_task_id: task.id.clone(),
                    },
                )?;
                (task.id, attempt.id)
            }
            _ => return Err(CleanStoreError::InvalidTransition),
        };

        Ok((task, attempt))
    }
    fn admit_project_proposal(
        &mut self,
        stable_key: &str,
        p: &v1::ProjectTaskProposal,
    ) -> Result<(String, String), CleanStoreError> {
        let request = p.request.clone().ok_or(CleanStoreError::CorruptState)?;
        self.validate_proposal_admission(p, &request)?;
        let command = crate::adapters::grpc::task_mapping::task_command(
            stable_key,
            &v1::CreateDevelopmentTaskRequest {
                idempotency_key: stable_key.into(),
                request: Some(request),
            },
        )?;
        let receipt = ayni_orchestrator_application::tasks::admit(self, command)
            .map_err(super::task_admission::admission_error)?;
        Ok((receipt.task_id, receipt.attempt_id))
    }
    fn validate_proposal_admission(
        &self,
        p: &v1::ProjectTaskProposal,
        request: &v1::DevelopmentTaskRequest,
    ) -> Result<(), CleanStoreError> {
        self.project_member(&request.project_id, &request.repository_connection_id)?;
        let repository = self.repository_connection(&request.repository_connection_id)?;
        if !repository.onboarding_revision.is_empty()
            && repository.onboarding_revision != request.repository_revision
        {
            return Err(CleanStoreError::StaleVersion);
        }
        let frozen = p
            .researched_runtime
            .as_ref()
            .ok_or(CleanStoreError::ProfileNotReady)?;
        let profile = self.runtime_profile(&request.runtime_profile_id)?;
        if profile.active_revision_id.as_deref()
            != Some(frozen.runtime_profile_revision_id.as_str())
            || profile.active_runtime_artifact_id.as_deref()
                != Some(frozen.runtime_artifact_id.as_str())
            || self
                .runtime_artifact(&frozen.runtime_artifact_id)?
                .resolved_digest
                != frozen.resolved_digest
        {
            return Err(CleanStoreError::StaleVersion);
        }
        let latest:Option<String>=self.connection.query_row("SELECT revision FROM repository_snapshots WHERE repository_connection_id=?1 ORDER BY event_cursor DESC LIMIT 1",[&request.repository_connection_id],|r|r.get(0)).optional().map_err(CleanStoreError::Database)?;
        if latest.as_deref() != Some(request.repository_revision.as_str()) {
            return Err(CleanStoreError::StaleVersion);
        }
        Ok(())
    }
}
pub(super) fn validate_conversation_index(
    c: &v1::ProjectConversation,
    id: &str,
    project: &str,
    cursor: i64,
    payload_bytes: usize,
) -> Result<(), CleanStoreError> {
    if c.id != id
        || c.project_id != project
        || cursor <= 0
        || u64::try_from(cursor).ok() != Some(c.update_cursor)
        || payload_bytes > 3 * 1024 * 1024
        || c.messages.len() > 512
    {
        return Err(CleanStoreError::CorruptState);
    }
    Ok(())
}
pub(super) fn validate_conversation_runs(
    c: &v1::ProjectConversation,
) -> Result<(), CleanStoreError> {
    let mut ids = std::collections::BTreeSet::new();
    for run in &c.runs {
        if !ids.insert(run.id.clone())
            || !matches!(
                run.state.as_str(),
                "queued" | "running" | "completed" | "interrupted" | "failed"
            )
        {
            return Err(CleanStoreError::CorruptState);
        }
    }
    if c.runs
        .iter()
        .filter(|r| matches!(r.state.as_str(), "queued" | "running"))
        .count()
        > 1
    {
        return Err(CleanStoreError::CorruptState);
    }
    for proposal in &c.proposals {
        if !ids.contains(&proposal.run_id)
            || !matches!(
                proposal.state.as_str(),
                "pending" | "approved" | "rejected" | "revise" | "failed"
            )
        {
            return Err(CleanStoreError::CorruptState);
        }
    }
    Ok(())
}
fn validate_project_result(
    c: &v1::ProjectConversation,
    result: &ProjectAgentResult,
    researched: Option<&ProjectAgentRequest>,
) -> Result<(), CleanStoreError> {
    if result.message.len() > 16384
        || result.proposals.len() > c.repository_connection_ids.len()
        || result.model_calls > researched.map_or(24, |r| r.max_model_calls)
    {
        return Err(CleanStoreError::BoundExceeded);
    }
    if c.runs
        .iter()
        .any(|r| r.id == result.run_id && r.purpose == "summary")
        && !result.proposals.is_empty()
    {
        return Err(CleanStoreError::InvalidTransition);
    }
    Ok(())
}
fn proposal_request(
    project_id: &str,
    p: &ResearchProposal,
) -> Result<v1::DevelopmentTaskRequest, CleanStoreError> {
    Ok(v1::DevelopmentTaskRequest {
        project_id: project_id.into(),
        repository_connection_id: p.repository_id.clone(),
        repository_revision: p.revision.clone(),
        runtime_profile_id: p.runtime_profile_id.clone(),
        agent: Some(v1::AgentSelection {
            provider: "openai".into(),
            agent: "coding-agent".into(),
            model: p.coding_model.clone(),
        }),
        request: if p.allowed_paths.is_empty() {
            p.request.clone()
        } else {
            format!(
                "{}\nAllowed paths: {}",
                p.request,
                p.allowed_paths.join(", ")
            )
        },
        acceptance_criteria: p.acceptance_criteria.clone(),
        change_constraints: Some(v1::ChangeConstraints {
            protected_paths: p.forbidden_paths.clone(),
            allow_test_edits: p.allow_test_edits,
        }),
        coding_turns: Some(p.coding_turns),
        deadline: Some(prost_types::Duration {
            seconds: i64::try_from(p.deadline_seconds)
                .map_err(|_| CleanStoreError::BoundExceeded)?,
            nanos: 0,
        }),
        connection_id: p.coding_connection_id.clone(),
    })
}
fn finish_transaction(
    connection: &Connection,
    result: Result<(), CleanStoreError>,
) -> Result<(), CleanStoreError> {
    match result {
        Ok(()) => connection
            .execute_batch("COMMIT")
            .map_err(CleanStoreError::Database),
        Err(error) => {
            let _ = connection.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}
fn api_validation_task(request: &v1::DevelopmentTaskRequest) -> Result<(), CleanStoreError> {
    ayni_orchestrator_api::validation::create_development_task(&v1::CreateDevelopmentTaskRequest {
        idempotency_key: "project-agent-proposal".into(),
        request: Some(request.clone()),
    })
    .map_err(|_| CleanStoreError::BoundExceeded)
}
