//! `SQLite` owns reusable planner leases; resident workers are disposable caches.
use super::{
    CleanStore, CleanStoreError, Connection, Message, OptionalExtension, decode, params,
    resource_id, timestamp, unix_seconds, v1,
};
use serde::{Deserialize, Serialize};

pub(crate) const DEFAULT_IDLE_SECONDS: u32 = 300;
pub(crate) const fn idle_timeout(value: u32) -> Result<u32, CleanStoreError> {
    if matches!(value, 300 | 86400) {
        Ok(value)
    } else {
        Err(CleanStoreError::InvalidTransition)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PlannerSession {
    pub conversation_id: String,
    pub session_id: String,
    pub generation: u64,
    pub state: String,
    pub active_run_id: String,
    pub turn_sequence: u64,
    pub last_activity_at: i64,
    pub idle_started_at: Option<i64>,
    pub idle_expires_at: Option<i64>,
    pub pod_uid: String,
    pub model_secret: String,
    pub model_uid: String,
    pub model_revision: String,
    #[serde(default)]
    pub lost_after_stop: bool,
    #[serde(default)]
    pub pending_run_id: String,
}
impl PlannerSession {
    pub(crate) fn projection(&self) -> v1::ProjectPlannerSession {
        v1::ProjectPlannerSession {
            session_id: self.session_id.clone(),
            generation: self.generation,
            state: self.state.clone(),
            last_activity_at: Some(timestamp(self.last_activity_at)),
            idle_expires_at: self.idle_expires_at.map(timestamp),
            active_run_id: self.active_run_id.clone(),
        }
    }
}
pub(super) fn migrate(connection: &Connection) -> Result<(), CleanStoreError> {
    connection
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(CleanStoreError::Database)?;
    let result = (|| {
        connection.execute_batch("CREATE TABLE project_planner_sessions(conversation_id TEXT PRIMARY KEY REFERENCES project_conversations(id),payload TEXT NOT NULL);").map_err(CleanStoreError::Database)?;
        let rows = connection
            .prepare("SELECT payload FROM workspaces")
            .map_err(CleanStoreError::Database)?
            .query_map([], |r| r.get::<_, Vec<u8>>(0))
            .map_err(CleanStoreError::Database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(CleanStoreError::Database)?;
        for bytes in rows {
            let mut w: v1::Workspace = decode(&bytes)?;
            w.planner_idle_timeout_seconds = DEFAULT_IDLE_SECONDS;
            connection
                .execute(
                    "UPDATE workspaces SET payload=?1 WHERE id=?2",
                    params![w.encode_to_vec(), w.id],
                )
                .map_err(CleanStoreError::Database)?;
        }
        // Legacy runs and frozen input bytes are intentionally unchanged.
        connection
            .execute_batch("UPDATE clean_schema SET version=23 WHERE component='control_plane'")
            .map_err(CleanStoreError::Database)
    })();
    finish(connection, result)
}
fn finish<T>(
    connection: &Connection,
    result: Result<T, CleanStoreError>,
) -> Result<T, CleanStoreError> {
    match result {
        Ok(value) => connection
            .execute_batch("COMMIT")
            .map(|()| value)
            .map_err(CleanStoreError::Database),
        Err(e) => {
            let _ = connection.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}
impl CleanStore {
    pub(crate) fn planner_session(
        &self,
        id: &str,
    ) -> Result<Option<PlannerSession>, CleanStoreError> {
        let payload: Option<String> = self
            .connection
            .query_row(
                "SELECT payload FROM project_planner_sessions WHERE conversation_id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()
            .map_err(CleanStoreError::Database)?;
        payload
            .map(|p| serde_json::from_str(&p).map_err(|_| CleanStoreError::CorruptState))
            .transpose()
    }
    fn save_planner_session(&self, s: &PlannerSession) -> Result<(), CleanStoreError> {
        let payload = serde_json::to_string(s).map_err(|_| CleanStoreError::CorruptState)?;
        self.connection.execute("INSERT INTO project_planner_sessions VALUES(?1,?2) ON CONFLICT(conversation_id) DO UPDATE SET payload=excluded.payload",params![s.conversation_id,payload]).map_err(CleanStoreError::Database)?;
        Ok(())
    }
    pub(crate) fn planner_sessions_page(
        &self,
        after: &str,
    ) -> Result<Vec<PlannerSession>, CleanStoreError> {
        let payloads = self
            .connection
            .prepare(
                "SELECT payload FROM project_planner_sessions WHERE conversation_id>?1 ORDER BY conversation_id LIMIT 128",
            )
            .map_err(CleanStoreError::Database)?
            .query_map([after], |r| r.get::<_, String>(0))
            .map_err(CleanStoreError::Database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(CleanStoreError::Database)?;
        payloads
            .into_iter()
            .map(|p| serde_json::from_str(&p).map_err(|_| CleanStoreError::CorruptState))
            .collect()
    }
    pub(super) fn conversation_session_projection(
        &self,
        c: &mut v1::ProjectConversation,
    ) -> Result<(), CleanStoreError> {
        let project = self.project(&c.project_id)?;
        let workspace = self.workspace(&project.workspace_id)?;
        c.effective_planner_idle_timeout_seconds =
            idle_timeout(c.planner_idle_timeout_seconds.unwrap_or(
                if workspace.planner_idle_timeout_seconds == 0 {
                    DEFAULT_IDLE_SECONDS
                } else {
                    workspace.planner_idle_timeout_seconds
                },
            ))?;
        c.planner_session = self.planner_session(&c.id)?.map(|mut s| {
            if s.state == "idle" {
                s.idle_expires_at = s
                    .idle_started_at
                    .map(|at| at + i64::from(c.effective_planner_idle_timeout_seconds));
            }
            s.projection()
        });
        Ok(())
    }
    pub(super) fn note_planner_message(
        &self,
        c: &v1::ProjectConversation,
        now: i64,
    ) -> Result<(), CleanStoreError> {
        if let Some(mut s) = self.planner_session(&c.id)? {
            s.last_activity_at = now;
            if s.state == "idle" {
                let expired = s.idle_started_at.is_none_or(|at| {
                    at + i64::from(c.effective_planner_idle_timeout_seconds) <= now
                });
                if expired {
                    s.state = "stopping".into();
                    s.idle_expires_at = None;
                    s.pending_run_id.clear();
                } else {
                    s.idle_started_at = Some(now);
                    s.idle_expires_at =
                        Some(now + i64::from(c.effective_planner_idle_timeout_seconds));
                    if let Some(run) = c.runs.last() {
                        s.pending_run_id.clone_from(&run.id);
                    }
                }
            }
            self.save_planner_session(&s)?;
        }
        Ok(())
    }
    fn scoped_conversation(
        &self,
        project: &str,
        id: &str,
    ) -> Result<v1::ProjectConversation, CleanStoreError> {
        let c = self.project_conversation(id)?;
        if c.project_id != project {
            return Err(CleanStoreError::ParentMismatch);
        }
        for repo in &c.repository_connection_ids {
            self.project_member(project, repo)?;
        }
        Ok(c)
    }
    pub(crate) fn update_project_conversation_settings(
        &mut self,
        key: &str,
        r: &v1::UpdateProjectConversationSettingsRequest,
    ) -> Result<v1::ProjectConversation, CleanStoreError> {
        if let Some(c) = self.replay_conversation(key, r)? {
            return Ok(c);
        }
        self.connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(CleanStoreError::Database)?;
        let result = (|| {
            if let Some(c) = self.replay_conversation(key, r)? {
                return Ok(c);
            }
            let mut c = self.scoped_conversation(&r.project_id, &r.conversation_id)?;
            if c.update_cursor != r.expected_update_cursor {
                return Err(CleanStoreError::StaleVersion);
            }
            if r.inherit_workspace_timeout && r.planner_idle_timeout_seconds.is_some() {
                return Err(CleanStoreError::InvalidTransition);
            }
            if let Some(timeout) = r.planner_idle_timeout_seconds {
                idle_timeout(timeout)?;
                c.planner_idle_timeout_seconds = Some(timeout);
            } else if r.inherit_workspace_timeout {
                c.planner_idle_timeout_seconds = None;
            } else {
                return Err(CleanStoreError::InvalidTransition);
            }

            self.conversation_session_projection(&mut c)?;
            if let Some(mut s) = self.planner_session(&c.id)?
                && s.state == "idle"
            {
                s.idle_expires_at = s
                    .idle_started_at
                    .map(|at| at + i64::from(c.effective_planner_idle_timeout_seconds));
                self.save_planner_session(&s)?;
            }
            self.save_conversation(&mut c)?;
            self.remember_conversation(key, r, &c.id)?;
            Ok(c)
        })();
        finish(&self.connection, result)
    }
    pub(crate) fn cancel_project_agent_run(
        &mut self,
        key: &str,
        r: &v1::CancelProjectAgentRunRequest,
    ) -> Result<v1::ProjectConversation, CleanStoreError> {
        if let Some(c) = self.replay_conversation(key, r)? {
            return Ok(c);
        }
        self.connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(CleanStoreError::Database)?;
        let result = (|| {
            if let Some(c) = self.replay_conversation(key, r)? {
                return Ok(c);
            }
            let mut c = self.project_conversation(&r.conversation_id)?;
            if c.project_id != r.project_id {
                return Err(CleanStoreError::ParentMismatch);
            }
            let run = c
                .runs
                .iter_mut()
                .find(|run| run.id == r.run_id)
                .ok_or(CleanStoreError::InvalidTransition)?;
            if !matches!(run.state.as_str(), "queued" | "running" | "cancelled") {
                return Err(CleanStoreError::InvalidTransition);
            }
            run.state = "cancelled".into();
            run.failure = "cancelled by operator".into();

            if let Some(mut s) = self.planner_session(&c.id)?
                && s.active_run_id == r.run_id
            {
                s.state = "stopping".into();
                s.active_run_id.clear();
                s.pending_run_id.clear();
                s.idle_expires_at = None;
                self.save_planner_session(&s)?;
            }
            self.save_conversation(&mut c)?;
            self.remember_conversation(key, r, &c.id)?;
            Ok(c)
        })();
        finish(&self.connection, result)
    }
    /// Assign once before checkout. An incompatible idle resident is stopped before replacement.
    pub(crate) fn prepare_planner_session(
        &mut self,
        conversation_id: &str,
        run_id: &str,
        deadline_seconds: u64,
    ) -> Result<Option<(PlannerSession, v1::ProjectAgentRun)>, CleanStoreError> {
        self.connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(CleanStoreError::Database)?;
        let mut answer = None;
        let result = (|| {
            let mut c = self.project_conversation(conversation_id)?;
            let index = c
                .runs
                .iter()
                .position(|r| {
                    r.id == run_id
                        && r.session_managed
                        && matches!(r.state.as_str(), "queued" | "running")
                })
                .ok_or(CleanStoreError::StaleVersion)?;
            let run = &mut c.runs[index];
            let binding = run
                .provider_credentials
                .as_ref()
                .ok_or(CleanStoreError::InvalidTransition)?;
            let now = unix_seconds()?;
            let existing = self.planner_session(conversation_id)?;
            if let Some(s) = &existing {
                if s.state == "stopping" {
                    return Ok(());
                }
                if s.active_run_id == run_id
                    && s.generation == run.session_generation
                    && s.turn_sequence == run.session_turn_sequence
                {
                    answer = Some((s.clone(), run.clone()));
                    return Ok(());
                }
                if !s.active_run_id.is_empty() {
                    return Err(CleanStoreError::StaleVersion);
                }
            }
            let compatible = existing.as_ref().is_some_and(|s| {
                (s.state == "starting"
                    || (s.state == "idle"
                        && (s.pending_run_id == run_id
                            || s.idle_started_at.is_some_and(|at| {
                                at + i64::from(c.effective_planner_idle_timeout_seconds) > now
                            }))))
                    && s.model_secret == binding.secret_name
                    && s.model_uid == binding.secret_uid
                    && s.model_revision == binding.credential_revision
            });
            if !compatible
                && let Some(mut s) = existing.clone()
                && !matches!(s.state.as_str(), "expired" | "lost")
            {
                s.state = "stopping".into();
                s.idle_expires_at = None;
                self.save_planner_session(&s)?;
                self.save_conversation(&mut c)?;
                return Ok(());
            }
            let mut s = if compatible {
                existing.ok_or(CleanStoreError::CorruptState)?
            } else {
                PlannerSession {
                    conversation_id: conversation_id.into(),
                    session_id: resource_id("session"),
                    generation: existing
                        .as_ref()
                        .map_or(Some(1), |s| s.generation.checked_add(1))
                        .ok_or(CleanStoreError::BoundExceeded)?,
                    state: "starting".into(),
                    active_run_id: String::new(),
                    turn_sequence: 0,
                    last_activity_at: now,
                    idle_started_at: None,
                    idle_expires_at: None,
                    pod_uid: String::new(),
                    model_secret: binding.secret_name.clone(),
                    model_uid: binding.secret_uid.clone(),
                    model_revision: binding.credential_revision.clone(),
                    lost_after_stop: false,
                    pending_run_id: String::new(),
                }
            };
            s.turn_sequence = s
                .turn_sequence
                .checked_add(1)
                .ok_or(CleanStoreError::BoundExceeded)?;
            s.active_run_id = run_id.into();
            s.pending_run_id.clear();
            s.state = if s.pod_uid.is_empty() {
                "starting"
            } else {
                "active"
            }
            .into();
            s.idle_expires_at = None;
            s.idle_started_at = None;
            s.last_activity_at = now;
            run.session_id.clone_from(&s.session_id);
            run.session_generation = s.generation;
            run.session_turn_sequence = s.turn_sequence;
            run.session_deadline_at = Some(timestamp(
                now.checked_add(
                    i64::try_from(deadline_seconds).map_err(|_| CleanStoreError::BoundExceeded)?,
                )
                .ok_or(CleanStoreError::BoundExceeded)?,
            ));
            let bound = run.clone();
            self.save_planner_session(&s)?;
            self.save_conversation(&mut c)?;
            answer = Some((s, bound));
            Ok(())
        })();
        finish(&self.connection, result)?;
        Ok(answer)
    }
    pub(crate) fn bind_planner_pod(
        &mut self,
        conversation_id: &str,
        generation: u64,
        uid: &str,
    ) -> Result<(), CleanStoreError> {
        self.connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(CleanStoreError::Database)?;
        let result = (|| {
            let mut s = self
                .planner_session(conversation_id)?
                .ok_or(CleanStoreError::StaleVersion)?;
            if s.generation != generation
                || !matches!(s.state.as_str(), "starting" | "active")
                || (!s.pod_uid.is_empty() && s.pod_uid != uid)
            {
                return Err(CleanStoreError::StaleVersion);
            }
            s.pod_uid = uid.into();
            s.state = "active".into();
            self.save_planner_session(&s)
        })();
        finish(&self.connection, result)
    }
    pub(crate) fn bind_project_run_provider(
        &mut self,
        conversation_id: &str,
        run_id: &str,
        binding: v1::FrozenProviderCredentials,
    ) -> Result<(), CleanStoreError> {
        self.connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(CleanStoreError::Database)?;
        let result = (|| {
            let mut c = self.project_conversation(conversation_id)?;
            let run = c
                .runs
                .iter_mut()
                .find(|r| r.id == run_id && matches!(r.state.as_str(), "queued" | "running"))
                .ok_or(CleanStoreError::StaleVersion)?;
            if run.provider_credentials.is_some() {
                return Ok(());
            }
            if run.connection_id != binding.connection_id {
                return Err(CleanStoreError::StaleVersion);
            }
            run.credential_revision
                .clone_from(&binding.credential_revision);
            run.provider_credentials = Some(binding);
            self.save_conversation(&mut c)
        })();
        finish(&self.connection, result)
    }
    pub(crate) fn complete_planner_turn(
        &mut self,
        conversation_id: &str,
        run_id: &str,
        generation: u64,
        sequence: u64,
        result: ayni_orchestrator_runtime::project_agent::ProjectAgentResult,
    ) -> Result<(), CleanStoreError> {
        self.connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(CleanStoreError::Database)?;
        let done = (|| {
            let mut s = self
                .planner_session(conversation_id)?
                .ok_or(CleanStoreError::StaleVersion)?;
            let c = self.project_conversation(conversation_id)?;
            if c.runs
                .iter()
                .any(|r| r.id == run_id && r.state == "completed")
            {
                return Ok(());
            }
            if s.generation != generation
                || s.turn_sequence != sequence
                || s.active_run_id != run_id
                || !matches!(s.state.as_str(), "active" | "starting")
                || result.run_id != run_id
            {
                return Err(CleanStoreError::StaleVersion);
            }
            self.complete_project_run(result)?;
            s.state = "idle".into();
            s.active_run_id.clear();
            s.pending_run_id.clear();
            let now = unix_seconds()?;
            s.last_activity_at = now;
            s.idle_started_at = Some(now);
            s.idle_expires_at = Some(now + i64::from(c.effective_planner_idle_timeout_seconds));
            self.save_planner_session(&s)?;
            let mut c = self.project_conversation(conversation_id)?;
            self.save_conversation(&mut c)
        })();
        finish(&self.connection, done)
    }
    pub(crate) fn stop_planner_session(
        &mut self,
        conversation_id: &str,
        generation: u64,
        lost: bool,
    ) -> Result<(), CleanStoreError> {
        self.connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(CleanStoreError::Database)?;
        let result = (|| {
            let mut s = self
                .planner_session(conversation_id)?
                .ok_or(CleanStoreError::StaleVersion)?;
            if s.generation != generation {
                return Err(CleanStoreError::StaleVersion);
            }
            if !s.active_run_id.is_empty() {
                self.fail_project_run(
                    &s.active_run_id,
                    if lost {
                        "project_execution_lost"
                    } else {
                        "project_session_stopped"
                    },
                )?;
            }
            s.active_run_id.clear();
            s.pending_run_id.clear();
            s.state = if lost { "lost" } else { "expired" }.into();
            s.idle_expires_at = None;
            self.save_planner_session(&s)?;
            let mut c = self.project_conversation(conversation_id)?;
            self.save_conversation(&mut c)
        })();
        finish(&self.connection, result)
    }
    pub(crate) fn expire_planner_session(
        &mut self,
        conversation_id: &str,
        generation: u64,
        now: i64,
    ) -> Result<bool, CleanStoreError> {
        self.connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(CleanStoreError::Database)?;
        let mut expired = false;
        let result = (|| {
            let mut s = self
                .planner_session(conversation_id)?
                .ok_or(CleanStoreError::StaleVersion)?;
            let mut c = self.project_conversation(conversation_id)?;
            if s.state == "idle" {
                let expiry = s
                    .idle_started_at
                    .map(|at| at + i64::from(c.effective_planner_idle_timeout_seconds));
                if s.idle_expires_at != expiry {
                    s.idle_expires_at = expiry;
                    self.save_planner_session(&s)?;
                    self.save_conversation(&mut c)?;
                }
            }
            if s.generation == generation
                && s.state == "idle"
                && s.active_run_id.is_empty()
                && s.idle_expires_at.is_some_and(|at| at <= now)
                && !c
                    .runs
                    .iter()
                    .any(|r| matches!(r.state.as_str(), "queued" | "running"))
            {
                s.state = "stopping".into();
                s.idle_expires_at = None;
                self.save_planner_session(&s)?;
                self.save_conversation(&mut c)?;
                expired = true;
            }
            Ok(())
        })();
        finish(&self.connection, result)?;
        Ok(expired)
    }
    pub(super) fn validate_planner_sessions(&self) -> Result<(), CleanStoreError> {
        let mut after = String::new();
        loop {
            let page = self.planner_sessions_page(&after)?;
            if page.is_empty() {
                break;
            }
            after.clone_from(
                &page
                    .last()
                    .ok_or(CleanStoreError::CorruptState)?
                    .conversation_id,
            );
            for s in page {
                let c = self.project_conversation(&s.conversation_id)?;
                if s.generation == 0
                    || s.session_id.is_empty()
                    || !matches!(
                        s.state.as_str(),
                        "starting" | "active" | "idle" | "stopping" | "expired" | "lost"
                    )
                    || (!s.active_run_id.is_empty()
                        && !c.runs.iter().any(|r| {
                            r.id == s.active_run_id
                                && r.session_generation == s.generation
                                && r.session_id == s.session_id
                                && matches!(r.state.as_str(), "queued" | "running")
                        }))
                {
                    return Err(CleanStoreError::CorruptState);
                }
            }
        }
        Ok(())
    }
}

impl CleanStore {
    pub(crate) fn interrupt_planner_turn(
        &mut self,
        conversation_id: &str,
        run_id: &str,
        generation: u64,
        reason: &str,
    ) -> Result<(), CleanStoreError> {
        self.connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(CleanStoreError::Database)?;
        let result = (|| {
            let mut s = self
                .planner_session(conversation_id)?
                .ok_or(CleanStoreError::StaleVersion)?;
            if s.generation != generation || s.active_run_id != run_id {
                return Err(CleanStoreError::StaleVersion);
            }
            self.fail_project_run(run_id, reason)?;
            s.state = "stopping".into();
            s.active_run_id.clear();
            s.pending_run_id.clear();
            s.idle_expires_at = None;
            self.save_planner_session(&s)?;
            let mut c = self.project_conversation(conversation_id)?;
            self.save_conversation(&mut c)
        })();
        finish(&self.connection, result)
    }
}

impl CleanStore {
    pub(crate) fn retire_idle_planner_session(
        &mut self,
        id: &str,
        generation: u64,
    ) -> Result<(), CleanStoreError> {
        self.connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(CleanStoreError::Database)?;
        let result = (|| {
            let mut s = self
                .planner_session(id)?
                .ok_or(CleanStoreError::StaleVersion)?;
            if s.generation != generation || s.state != "idle" || !s.active_run_id.is_empty() {
                return Err(CleanStoreError::StaleVersion);
            }
            s.state = "stopping".into();
            s.lost_after_stop = true;
            s.idle_expires_at = None;
            self.save_planner_session(&s)?;
            let mut c = self.project_conversation(id)?;
            self.save_conversation(&mut c)
        })();
        finish(&self.connection, result)
    }
}
