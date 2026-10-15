//! Transport orchestration for serialized resident planner turns.
use super::{ProjectAgentReconciler, github, job_name};
use crate::{
    adapters::sqlite::{CleanStoreError, project_sessions::PlannerSession},
    coordinators::with_store_async,
};
use ayni_orchestrator_api::v1;
use ayni_orchestrator_kubernetes::project_agent::ProjectObservation;
use ayni_orchestrator_runtime::project_agent::ProjectSessionTurn;
use std::collections::BTreeMap;

impl ProjectAgentReconciler {
    pub(super) async fn reconcile_sessions(&self) -> Result<(), &'static str> {
        let mut after = String::new();
        loop {
            let cursor = after.clone();
            let sessions = with_store_async(&self.state, move |s| s.planner_sessions_page(&cursor))
                .await
                .map_err(|_| "project_store_unavailable")?;
            if sessions.is_empty() {
                break;
            }
            after.clone_from(
                &sessions
                    .last()
                    .ok_or("project_session_invalid")?
                    .conversation_id,
            );
            for session in sessions {
                let id = session.conversation_id.clone();
                if let Err(reason) = self.reconcile_session(session).await {
                    tracing::warn!(
                        conversation_id = id,
                        reason_code = reason,
                        "project session cleanup deferred"
                    );
                }
            }
        }
        Ok(())
    }
    async fn reconcile_session(&self, mut session: PlannerSession) -> Result<(), &'static str> {
        if session.state == "idle"
            && !session.pod_uid.is_empty()
            && !self
                .kubernetes
                .project_session_alive(&job_name(&session.session_id), &session.pod_uid)
                .await?
        {
            let id = session.conversation_id.clone();
            let generation = session.generation;
            with_store_async(&self.state, move |s| {
                s.retire_idle_planner_session(&id, generation)
            })
            .await
            .map_err(|_| "project_store_unavailable")?;
            session.state = "stopping".into();
            session.lost_after_stop = true;
        }
        if session.state == "idle" {
            let id = session.conversation_id.clone();
            let generation = session.generation;
            let expired = with_store_async(&self.state, move |s| {
                s.expire_planner_session(&id, generation, crate::adapters::sqlite::unix_seconds()?)
            })
            .await
            .map_err(|_| "project_store_unavailable")?;
            if expired {
                session.state = "stopping".into();
            }
        }
        if session.state == "stopping" {
            let stopped = self
                .kubernetes
                .cleanup_project_session(&job_name(&session.session_id), &session.pod_uid)
                .await?;
            if stopped {
                let id = session.conversation_id;
                let generation = session.generation;
                let lost = session.lost_after_stop;
                with_store_async(&self.state, move |s| {
                    s.stop_planner_session(&id, generation, lost)
                })
                .await
                .map_err(|_| "project_store_unavailable")?;
            }
        }
        Ok(())
    }
    pub(super) async fn run_session(
        &self,
        c: &v1::ProjectConversation,
        run: &v1::ProjectAgentRun,
    ) -> Result<(), &'static str> {
        let mut run = run.clone();
        if !self.bind_session_provider(c, &mut run).await? {
            return Ok(());
        }
        let request = match self.request(c, &run).await {
            Ok(request) => request,
            Err(reason) => {
                let id = c.id.clone();
                let existing = with_store_async(&self.state, move |s| s.planner_session(&id))
                    .await
                    .map_err(|_| "project_store_unavailable")?;
                return if let Some(session) = existing.filter(|s| s.active_run_id == run.id) {
                    self.fail_session(&session, &run.id, reason).await
                } else {
                    self.fail(&run.id, reason).await
                };
            }
        };
        let id = c.id.clone();
        let run_id = run.id.clone();
        let deadline = request.deadline_seconds;
        let Some((mut session, bound)) = with_store_async(&self.state, move |s| {
            s.prepare_planner_session(&id, &run_id, deadline)
        })
        .await
        .map_err(|_| "project_session_unavailable")?
        else {
            return Ok(());
        };
        let deadline_at = bound
            .session_deadline_at
            .as_ref()
            .ok_or("project_session_deadline_missing")?
            .seconds;
        if turn_now()? >= deadline_at && session.pod_uid.is_empty() {
            return self
                .fail_session(&session, &bound.id, "project_turn_deadline_expired")
                .await;
        }
        let name = job_name(&session.session_id);
        match self
            .ensure_session_pod(c, &mut session, &name, &bound.id)
            .await
        {
            Ok(true) => {}
            Ok(false) => {
                return self
                    .unfinished_session(
                        &session,
                        &bound.id,
                        deadline_at,
                        "project_session_starting",
                    )
                    .await;
            }
            Err(reason) => {
                return self
                    .unfinished_session(&session, &bound.id, deadline_at, reason)
                    .await;
            }
        }
        let turn = ProjectSessionTurn {
            session_id: session.session_id.clone(),
            generation: session.generation,
            turn_sequence: bound.session_turn_sequence,
            pod_uid: session.pod_uid.clone(),
            deadline_at_unix_seconds: u64::try_from(deadline_at)
                .map_err(|_| "project_session_deadline_invalid")?,
            request,
        };
        let observation = self.kubernetes.observe_project_session(&name, &turn).await;
        let observation = apply_turn_deadline(observation, deadline_at, turn_now()?)?;
        if !matches!(observation, ProjectObservation::Running) {
            return self
                .accept_session_observation(c, &session, &bound, observation)
                .await;
        }
        if bound.state == "queued" {
            self.dispatch_session_turn(&name, &turn).await?;
        }
        Ok(())
    }
    async fn unfinished_session(
        &self,
        session: &PlannerSession,
        run_id: &str,
        deadline: i64,
        reason: &'static str,
    ) -> Result<(), &'static str> {
        match apply_turn_deadline(Err(reason), deadline, turn_now()?)? {
            ProjectObservation::Failed(reason) => self.fail_session(session, run_id, reason).await,
            _ => Err(reason),
        }
    }
    async fn dispatch_session_turn(
        &self,
        name: &str,
        turn: &ProjectSessionTurn,
    ) -> Result<(), &'static str> {
        // Re-submission after a crash is safe: the resident ledger owns paid-call deduplication.
        let mut tokens = BTreeMap::new();
        for repository in &turn.request.repositories {
            let id = repository.id.clone();
            let value = with_store_async(&self.state, move |s| s.repository_connection(&id))
                .await
                .map_err(|_| "project_repository_unavailable")?;
            let github = github(&value).ok_or("project_repository_unsupported")?;
            let source = self
                .state
                .source
                .as_ref()
                .ok_or("project_source_unconfigured")?;
            tokens.insert(
                ayni_orchestrator_runtime::project_agent::research_key(&repository.id),
                source
                    .token(github.installation_id, &github.repository_id)
                    .await?,
            );
        }
        if let Err(reason) = self
            .kubernetes
            .dispatch_project_session(name, turn, tokens)
            .await
        {
            // A transport disconnect may occur after acceptance; leave the same turn queued for ledger-safe replay.
            tracing::warn!(
                run_id = turn.request.run_id,
                reason_code = reason,
                "project session dispatch deferred"
            );
            return Ok(());
        }
        let id = turn.request.run_id.clone();
        with_store_async(&self.state, move |s| s.mark_project_run_running(&id))
            .await
            .map_err(|_| "project_store_unavailable")?;
        Ok(())
    }
    async fn bind_session_provider(
        &self,
        c: &v1::ProjectConversation,
        run: &mut v1::ProjectAgentRun,
    ) -> Result<bool, &'static str> {
        if run.provider_credentials.is_none() {
            let _guard = self.state.connection_lock.lock().await;
            let Ok(binding) = self
                .state
                .freeze_provider_credentials(&run.connection_id, &run.model)
                .await
            else {
                self.fail(&run.id, "project_model_binding_unavailable")
                    .await?;
                return Ok(false);
            };
            let id = c.id.clone();
            let run_id = run.id.clone();
            let saved = binding.clone();
            with_store_async(&self.state, move |s| {
                s.bind_project_run_provider(&id, &run_id, saved)
            })
            .await
            .map_err(|_| "project_store_unavailable")?;
            run.provider_credentials = Some(binding);
        }
        Ok(true)
    }
    async fn ensure_session_pod(
        &self,
        c: &v1::ProjectConversation,
        session: &mut PlannerSession,
        name: &str,
        run_id: &str,
    ) -> Result<bool, &'static str> {
        if session.pod_uid.is_empty() {
            let uid = match self
                .kubernetes
                .ensure_project_session(
                    name,
                    &session.session_id,
                    session.generation,
                    &session.model_secret,
                    &session.model_uid,
                    &session.model_revision,
                )
                .await
            {
                Ok(uid) => uid,
                Err(reason) => {
                    tracing::warn!(
                        run_id = run_id,
                        reason_code = reason,
                        "project session creation deferred"
                    );
                    return Ok(false);
                }
            };
            let id = c.id.clone();
            let generation = session.generation;
            let saved = uid.clone();
            with_store_async(&self.state, move |s| {
                s.bind_planner_pod(&id, generation, &saved)
            })
            .await
            .map_err(|_| "project_session_unavailable")?;
            session.pod_uid = uid;
        }
        Ok(true)
    }
    async fn accept_session_observation(
        &self,
        c: &v1::ProjectConversation,
        session: &PlannerSession,
        bound: &v1::ProjectAgentRun,
        observation: ProjectObservation,
    ) -> Result<(), &'static str> {
        match observation {
            ProjectObservation::Missing => {
                let id = c.id.clone();
                let generation = session.generation;
                with_store_async(&self.state, move |s| {
                    s.stop_planner_session(&id, generation, true)
                })
                .await
                .map_err(|_| "project_store_unavailable")
            }
            ProjectObservation::Running => Ok(()),
            ProjectObservation::Failed(reason) => {
                self.fail_session(session, &bound.id, reason).await
            }
            ProjectObservation::Completed(result) => {
                let id = c.id.clone();
                let run_id = bound.id.clone();
                let generation = session.generation;
                let sequence = bound.session_turn_sequence;
                match with_store_async(&self.state,move|s|s.complete_planner_turn(&id,&run_id,generation,sequence,result)).await {
                    Ok(())|Err(CleanStoreError::StaleVersion)=>Ok(()),
                    Err(CleanStoreError::ConversationCapacity)=>self.fail_session(session,&bound.id,"conversation capacity reached; start another conversation; original records are retained").await,
                    Err(_)=>self.fail_session(session,&bound.id,"project_result_rejected").await,
                }
            }
        }
    }
    async fn fail_session(
        &self,
        s: &PlannerSession,
        run_id: &str,
        reason: &'static str,
    ) -> Result<(), &'static str> {
        let id = s.conversation_id.clone();
        let generation = s.generation;
        let run_id = run_id.to_owned();
        match with_store_async(&self.state, move |store| {
            store.interrupt_planner_turn(&id, &run_id, generation, reason)
        })
        .await
        {
            Ok(()) | Err(CleanStoreError::StaleVersion) => Ok(()),
            Err(_) => Err("project_store_unavailable"),
        }
    }
}

fn turn_now() -> Result<i64, &'static str> {
    crate::adapters::sqlite::unix_seconds().map_err(|_| "project_clock_unavailable")
}
// A completed worker receipt survives a late controller restart. Unreachable or
// unfinished work still stops at its original deadline, including transport errors.
fn apply_turn_deadline(
    observation: Result<ProjectObservation, &'static str>,
    deadline: i64,
    now: i64,
) -> Result<ProjectObservation, &'static str> {
    match observation {
        Ok(ProjectObservation::Running) | Err(_) if now >= deadline => {
            Ok(ProjectObservation::Failed("project_turn_deadline_expired"))
        }
        other => other,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn polling_errors_and_unfinished_work_expire_but_matching_completed_receipts_survive() {
        assert!(matches!(
            apply_turn_deadline(Err("transport_unavailable"), 100, 100),
            Ok(ProjectObservation::Failed("project_turn_deadline_expired"))
        ));
        assert!(matches!(
            apply_turn_deadline(Ok(ProjectObservation::Running), 100, 100),
            Ok(ProjectObservation::Failed("project_turn_deadline_expired"))
        ));
        assert!(matches!(
            apply_turn_deadline(Err("transport_unavailable"), 100, 99),
            Err("transport_unavailable")
        ));
        let result = ayni_orchestrator_runtime::project_agent::ProjectAgentResult {
            run_id: "completed".into(),
            message: "Finished within worker budget".into(),
            proposals: vec![],
            model_calls: 1,
            token_usage: ayni_orchestrator_domain::ProviderTokenUsage::default(),
        };
        assert!(matches!(
            apply_turn_deadline(Ok(ProjectObservation::Completed(result)), 100, 150),
            Ok(ProjectObservation::Completed(_))
        ));
    }
}
