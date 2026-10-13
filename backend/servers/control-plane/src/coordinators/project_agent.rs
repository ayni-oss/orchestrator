//! Durable project-agent launch, observation and recovery lane.
use crate::{
    AppState,
    adapters::sqlite::{CleanStore, CleanStoreError},
    coordinators::with_store_async,
    process::runtime_configuration::RuntimeConfiguration,
};
use ayni_orchestrator_api::v1;
use ayni_orchestrator_kubernetes::{
    execution::RuntimeKubernetes, project_agent::ProjectObservation,
};
use ayni_orchestrator_runtime::project_agent::{ProjectAgentRequest, ResearchRepository};
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::sync::Mutex;

pub struct ProjectAgentReconciler {
    state: AppState,
    configuration: RuntimeConfiguration,
    kubernetes: RuntimeKubernetes,
    pass: Mutex<()>,
}
impl ProjectAgentReconciler {
    /// Build an optional in-cluster research lane from the runtime deployment policy.
    /// # Errors
    /// Returns a bounded error when configured infrastructure is unavailable.
    pub fn from_environment(state: AppState) -> Result<Option<Arc<Self>>, &'static str> {
        let Some(configuration) = RuntimeConfiguration::from_environment()? else {
            return Ok(None);
        };
        let kubernetes = RuntimeKubernetes::new(configuration.execution_configuration())?;
        Ok(Some(Arc::new(Self {
            state,
            configuration,
            kubernetes,
            pass: Mutex::new(()),
        })))
    }
    pub async fn reconcile(&self) {
        let Ok(_guard) = self.pass.try_lock() else {
            return;
        };
        if let Err(error) = self.reconcile_pass().await {
            tracing::warn!(
                reason_code = error,
                "project research reconciliation deferred"
            );
        }
    }
    async fn reconcile_pass(&self) -> Result<(), &'static str> {
        // Accepted attempt state, not broker delivery, owns summary eligibility.
        with_store_async(&self.state, CleanStore::queue_project_outcome_summaries)
            .await
            .map_err(|_| "project_summary_unavailable")?;
        let runs = with_store_async(&self.state, |s| s.queued_project_runs())
            .await
            .map_err(|_| "project_store_unavailable")?;
        for (conversation, run) in runs.into_iter().take(self.configuration.concurrency) {
            let result = tokio::time::timeout(
                Duration::from_secs(self.configuration.request_timeout_seconds * 4),
                self.run(&conversation, &run),
            )
            .await;
            if let Ok(Err(reason)) = result {
                tracing::warn!(
                    run_id = run.id,
                    reason_code = reason,
                    "project run observation deferred"
                );
            }
        }
        let terminal = with_store_async(&self.state, |s| s.terminal_project_runs())
            .await
            .map_err(|_| "project_store_unavailable")?;
        for (_, run) in terminal {
            let _ = self
                .kubernetes
                .cleanup_project_agent(&job_name(&run.id))
                .await;
        }
        Ok(())
    }
    async fn run(
        &self,
        conversation: &v1::ProjectConversation,
        run: &v1::ProjectAgentRun,
    ) -> Result<(), &'static str> {
        let name = job_name(&run.id);
        match self
            .kubernetes
            .observe_project_agent(&name, &run.id)
            .await?
        {
            ProjectObservation::Completed(result) => {
                if with_store_async(&self.state, move |s| s.complete_project_run(result))
                    .await
                    .is_err()
                {
                    return self.fail(&run.id, "project_result_rejected").await;
                }
                self.kubernetes.cleanup_project_agent(&name).await
            }
            ProjectObservation::Failed(reason) => self.fail(&run.id, reason).await,
            ProjectObservation::Running => {
                let id = run.id.clone();
                with_store_async(&self.state, move |s| s.mark_project_run_running(&id))
                    .await
                    .map_err(|_| "project_store_unavailable")
            }
            ProjectObservation::Missing if run.state == "running" => {
                self.fail(&run.id, "project_execution_lost").await
            }
            ProjectObservation::Missing => {
                let Ok(request) = self.request(conversation, run).await else {
                    return self
                        .fail(&run.id, "project_research_inputs_unavailable")
                        .await;
                };
                let binding = run
                    .provider_credentials
                    .as_ref()
                    .ok_or("project_model_binding_missing")?;
                let mut tokens = BTreeMap::new();
                for repository in &request.repositories {
                    let id = repository.id.clone();
                    let value =
                        with_store_async(&self.state, move |s| s.repository_connection(&id))
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
                self.kubernetes
                    .start_project_agent(
                        &name,
                        &request,
                        tokens,
                        &binding.secret_name,
                        &binding.secret_uid,
                        &binding.credential_revision,
                    )
                    .await?;
                let id = run.id.clone();
                with_store_async(&self.state, move |s| s.mark_project_run_running(&id))
                    .await
                    .map_err(|_| "project_store_unavailable")
            }
        }
    }
    async fn request(
        &self,
        conversation: &v1::ProjectConversation,
        run: &v1::ProjectAgentRun,
    ) -> Result<ProjectAgentRequest, &'static str> {
        let id = run.id.clone();
        let existing = with_store_async(&self.state, move |s| s.project_run_request(&id))
            .await
            .map_err(|_| "project_store_unavailable")?;
        if let Some(request) = existing {
            return Ok(request);
        }

        let mut source_snapshots = BTreeMap::new();
        if run.purpose != "summary" {
            for repository in &conversation.repository_connection_ids {
                let observed = self
                    .state
                    .observe_project_repository_head(&conversation.project_id, repository)
                    .await?;
                source_snapshots.insert(repository.clone(), observed);
            }
        }

        let project_id = conversation.project_id.clone();
        let project = with_store_async(&self.state, move |s| s.project(&project_id))
            .await
            .map_err(|_| "project_store_unavailable")?;
        let defaults = project
            .agent_defaults
            .as_ref()
            .ok_or("project_models_unconfigured")?;
        let catalog = self
            .state
            .catalog_for_connection(&defaults.coding_connection_id)
            .await
            .map_err(|_| "project_coding_provider_unavailable")?;
        let models = catalog
            .available()
            .await
            .map_err(|_| "project_models_unavailable")?;
        let models = serde_json::to_string(&models).map_err(|_| "project_models_invalid")?;
        let c = conversation.clone();
        let r = run.clone();
        let calls = self.configuration.project_agent_model_calls;
        let deadline = self.configuration.project_agent_deadline_seconds;
        let prepared = with_store_async(&self.state, move |s| {
            prepare_request(s, &c, &r, calls, deadline, &models, &source_snapshots)
        })
        .await;
        let request = prepared.map_err(|_| "project_research_inputs_unavailable")?;
        let id = run.id.clone();
        let saved = request.clone();
        with_store_async(&self.state, move |s| {
            s.save_project_run_request(&id, &saved)
        })
        .await
        .map_err(|_| "project_store_unavailable")?;
        Ok(request)
    }
    async fn fail(&self, id: &str, reason: &'static str) -> Result<(), &'static str> {
        let id = id.to_owned();
        with_store_async(&self.state, move |s| s.fail_project_run(&id, reason))
            .await
            .map_err(|_| "project_store_unavailable")
    }
}
impl AppState {
    /// Observe source with contents-read authority independently from runtime onboarding.
    /// Existing agent requests replay their own immutable source and do not call this.
    pub(crate) async fn observe_project_repository_head(
        &self,
        project_id: &str,
        repository_id: &str,
    ) -> Result<v1::RepositorySnapshot, &'static str> {
        let _lifecycle = self.repository_lifecycle.read().await;
        let project = project_id.to_owned();
        let repository = repository_id.to_owned();
        let value = with_store_async(self, move |store| {
            store.project_member(&project, &repository)?;
            store.repository_connection(&repository)
        })
        .await
        .map_err(|_| "project_repository_unavailable")?;
        let provider = github(&value).ok_or("project_repository_unsupported")?;
        let source = self.source.as_ref().ok_or("project_source_unconfigured")?;
        let token = source
            .token(provider.installation_id, &provider.repository_id)
            .await?;
        let revision = source
            .head(
                &provider.owner,
                &provider.name,
                &value.default_branch,
                &token,
            )
            .await?;
        let project = project_id.to_owned();
        let repository = repository_id.to_owned();
        with_store_async(self, move |store| {
            store.project_member(&project, &repository)?;
            store.observe_repository_snapshot(&repository, &revision)
        })
        .await
        .map_err(|_| "project_source_observation_unavailable")
    }
}

fn job_name(id: &str) -> String {
    format!(
        "project-{}",
        &format!("{:x}", sha2::Sha256::digest(id.as_bytes()))[..40]
    )
}
use sha2::Digest as _;
fn github(repository: &v1::RepositoryConnection) -> Option<&v1::GithubRepository> {
    match repository.provider.as_ref()?.provider.as_ref()? {
        v1::repository_provider::Provider::Github(g) => Some(g),
    }
}
fn prepare_request(
    store: &mut CleanStore,
    c: &v1::ProjectConversation,
    r: &v1::ProjectAgentRun,
    calls: u32,
    deadline: u64,
    models: &str,
    source_snapshots: &BTreeMap<String, v1::RepositorySnapshot>,
) -> Result<ProjectAgentRequest, CleanStoreError> {
    let project = store.project(&c.project_id)?;
    let defaults = project
        .agent_defaults
        .as_ref()
        .ok_or(CleanStoreError::InvalidTransition)?;
    let (repositories, context) = research_repositories(store, c, r, defaults, source_snapshots)?;
    let tasks = store.list_development_tasks(
        Some(&c.project_id),
        None,
        None,
        None,
        Some(&v1::PageRequest {
            page_size: 100,
            page_token: String::new(),
        }),
    )?;
    let mut task_context = Vec::new();
    for task in tasks.items {
        let attempts = store.list_development_attempts(
            &task.id,
            Some(&v1::PageRequest {
                page_size: 100,
                page_token: String::new(),
            }),
        )?;
        let Some(attempt) = attempts.items.into_iter().max_by_key(|a| a.attempt_number) else {
            continue;
        };
        task_context.push(json!({"id":task.id,"state":task.state,"repository_id":task.snapshot.as_ref().map(|s|s.repository.as_ref().map(|r|r.repository_connection_id.clone())),"attempt_id":attempt.id,"requested_input":attempt.requested_input,
            "outcome":format!("{:?}",attempt.result),"progress":format!("{:?}",attempt.progress)}));
    }
    let history = c
        .messages
        .iter()
        .rev()
        .take(32)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|m| format!("{}: {}", m.role, m.content))
        .collect::<Vec<_>>()
        .join("\n");
    let context=json!({"project_id":c.project_id,"repositories":context,"tasks":task_context,"available_coding_models":models,"purpose":r.purpose}).to_string();
    if history.len() > 131_072 || context.len() > 262_144 {
        return Err(CleanStoreError::BoundExceeded);
    }
    Ok(ProjectAgentRequest {
        run_id: r.id.clone(),
        project_id: c.project_id.clone(),
        model: r.model.clone(),
        history,
        repositories,
        context,
        max_model_calls: calls,
        deadline_seconds: deadline,
    })
}

fn research_repositories(
    store: &mut CleanStore,
    c: &v1::ProjectConversation,
    r: &v1::ProjectAgentRun,
    defaults: &v1::ProjectAgentDefaults,
    source_snapshots: &BTreeMap<String, v1::RepositorySnapshot>,
) -> Result<(Vec<ResearchRepository>, Vec<serde_json::Value>), CleanStoreError> {
    let mut repositories = Vec::new();
    let mut context = Vec::new();
    if r.purpose != "summary" {
        for id in &c.repository_connection_ids {
            store.project_member(&c.project_id, id)?;
            let repo = store.repository_connection(id)?;
            let provider = github(&repo).ok_or(CleanStoreError::InvalidTransition)?;
            let snapshot = source_snapshots
                .get(id)
                .ok_or(CleanStoreError::InvalidTransition)?;
            let profiles = store.list_runtime_profiles(
                Some(id),
                None,
                None,
                Some(&v1::PageRequest {
                    page_size: 100,
                    page_token: String::new(),
                }),
            )?;
            if !profiles.next_page_token.is_empty() {
                return Err(CleanStoreError::BoundExceeded);
            }
            let ready: Vec<_> = profiles
                .items
                .iter()
                .filter(|p| p.state == v1::RuntimeProfileState::Ready as i32)
                .collect();
            let selected = if ready.len() == 1 {
                ready[0].id.clone()
            } else {
                String::new()
            };
            let mut profile_context = Vec::new();
            for profile in &profiles.items {
                let artifact = profile
                    .active_runtime_artifact_id
                    .as_ref()
                    .map(|id| store.runtime_artifact(id))
                    .transpose()?;
                profile_context.push(json!({"id":profile.id,"name":profile.id,"state":profile.state,"ready":profile.state==v1::RuntimeProfileState::Ready as i32,
                    "active_revision_id":profile.active_revision_id,"active_runtime_artifact_id":profile.active_runtime_artifact_id,"resolved_digest":artifact.map(|a|a.resolved_digest)}));
            }
            context.push(json!({"repository_id":id,"display_name":repo.display_name,"revision":snapshot.revision,
                "profiles":profile_context }));
            repositories.push(ResearchRepository {
                id: id.clone(),
                repository_url: format!(
                    "https://github.com/{}/{}.git",
                    provider.owner, provider.name
                ),
                revision: snapshot.revision.clone(),
                coding_connection_id: defaults.coding_connection_id.clone(),
                coding_model: defaults.coding_model.clone(),
                runtime_profile_id: selected,
            });
        }
    }
    Ok((repositories, context))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain_repository_fixture() -> Result<
        (
            CleanStore,
            v1::RepositoryConnection,
            v1::ProjectConversation,
        ),
        CleanStoreError,
    > {
        let mut store = CleanStore::in_memory()?;
        let repository = store.create_repository_connection(
            "plain-repository",
            &v1::CreateRepositoryConnectionRequest {
                idempotency_key: "plain-repository".into(),
                display_name: "Read-only source".into(),
                default_branch: "main".into(),
                use_ayni: Some(false),
                provider: Some(v1::RepositoryProvider {
                    provider: Some(v1::repository_provider::Provider::Github(
                        v1::GithubRepository {
                            repository_id: "123".into(),
                            owner: "ayni-oss".into(),
                            name: "sandbox".into(),
                            installation_id: 42,
                            html_url: "https://github.com/ayni-oss/sandbox".into(),
                        },
                    )),
                }),
                ..Default::default()
            },
        )?;
        let project = store.create_project(
            "plain-project",
            &v1::CreateProjectRequest {
                idempotency_key: "plain-project".into(),
                display_name: "Plain research".into(),
                repository_connection_id: repository.id.clone(),
                repository_connection_ids: vec![],
            },
        )?;
        let conversation = store.create_project_conversation(
            "plain-conversation",
            &v1::CreateProjectConversationRequest {
                idempotency_key: "plain-conversation".into(),
                project_id: project.id,
                title: "Research before runtime setup".into(),
                repository_connection_ids: vec![],
            },
        )?;
        Ok((store, repository, conversation))
    }

    #[test]
    fn plain_repository_research_freezes_observed_source_without_any_runtime()
    -> Result<(), CleanStoreError> {
        let (mut store, repository, conversation) = plain_repository_fixture()?;
        let run = v1::ProjectAgentRun {
            id: "plain-run".into(),
            purpose: "research".into(),
            model: "research-model".into(),
            state: "queued".into(),
            ..Default::default()
        };
        store.send_project_message(
            "plain-message",
            &v1::SendProjectMessageRequest {
                idempotency_key: "plain-message".into(),
                conversation_id: conversation.id.clone(),
                content: "Inspect this repository".into(),
            },
            run.clone(),
        )?;
        let defaults = v1::ProjectAgentDefaults {
            project_connection_id: "project-provider".into(),
            project_model: "research-model".into(),
            coding_connection_id: "coding-provider".into(),
            coding_model: "coding-model".into(),
        };
        let revision = "a".repeat(40);
        assert!(matches!(
            store.repository_snapshot_for_revision(&repository.id, &revision),
            Err(CleanStoreError::NotFound { .. })
        ));
        assert!(
            research_repositories(&mut store, &conversation, &run, &defaults, &BTreeMap::new())
                .is_err()
        );
        // This is the durable seam after the read-only provider HEAD observation.
        let observed = store.observe_repository_snapshot(&repository.id, &revision)?;
        let source_snapshots = BTreeMap::from([(repository.id.clone(), observed)]);
        let (repositories, context) = research_repositories(
            &mut store,
            &conversation,
            &run,
            &defaults,
            &source_snapshots,
        )?;
        assert_eq!(repositories.len(), 1);
        assert_eq!(repositories[0].revision, revision);
        assert!(repositories[0].runtime_profile_id.is_empty());
        assert!(context[0]["profiles"].as_array().is_some_and(Vec::is_empty));
        let request = ProjectAgentRequest {
            run_id: run.id.clone(),
            project_id: conversation.project_id.clone(),
            model: run.model,
            history: "user: Inspect this repository".into(),
            repositories,
            context: json!({"repositories":context}).to_string(),
            max_model_calls: 24,
            deadline_seconds: 600,
        };
        store.save_project_run_request(&run.id, &request)?;
        store.observe_repository_snapshot(&repository.id, &"b".repeat(40))?;
        let frozen = store
            .project_run_request(&run.id)?
            .ok_or(CleanStoreError::CorruptState)?;
        assert_eq!(frozen.repositories[0].revision, revision);
        assert_eq!(frozen.context, request.context);
        assert_eq!(
            store.repository_connection(&repository.id)?.use_ayni,
            Some(false)
        );
        assert!(
            store
                .list_runtime_profiles(Some(&repository.id), None, None, None)?
                .items
                .is_empty()
        );
        Ok(())
    }
}
