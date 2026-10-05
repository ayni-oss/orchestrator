use super::{AppState, github, onboarding_templates, v1, with_store};
impl AppState {
    pub(super) async fn reconcile_setup_actions(&self) -> Result<(), &'static str> {
        let actions = with_store(self, move |s| {
            s.pending_external_actions("InstallRepositoryAutomation")
        })
        .await
        .map_err(|_| "setup_store_unavailable")?;
        // Open PRs retain pending intent. Rotate one action per pass so repositories
        // cannot starve each other or create a provider mutation burst.
        let offset = self
            .source
            .as_ref()
            .ok_or("github_app_unconfigured")?
            .setup_action_offset(actions.len());
        for action in actions.iter().cycle().skip(offset).take(1) {
            if let Err(reason) = self.reconcile_setup_action(action).await {
                tracing::warn!(reason_code = reason, "setup action deferred");
                if let Some(id) = &action.target_id {
                    let id = id.clone();
                    let _ = with_store(self, move |s| {
                        s.observe_setup(
                            &id,
                            None,
                            v1::AutomationInstallationState::Degraded,
                            Some(public_setup_failure(reason)),
                        )
                    })
                    .await;
                }
            }
        }
        Ok(())
    }

    pub(super) async fn reconcile_setup_action(
        &self,
        action: &crate::adapters::sqlite::DurableAction,
    ) -> Result<(), &'static str> {
        let id = action
            .target_id
            .as_deref()
            .ok_or("setup_identity_missing")?;
        let lookup_id = id.to_owned();
        let (installation, profile, repository) = with_store(self, move |s| {
            let installation = s.automation_installation(&lookup_id)?;
            let profile = s.runtime_profile(&installation.runtime_profile_id)?;
            let repository = s.repository_connection(&profile.repository_connection_id)?;
            Ok((installation, profile, repository))
        })
        .await
        .map_err(|_| "setup_store_unavailable")?;
        let github = github(&repository).map_err(|_| "setup_provider_unsupported")?;
        let source = self.source.as_ref().ok_or("github_app_unconfigured")?;
        let request = crate::adapters::github::setup::setup_pull_request(
            &profile.id,
            &repository.default_branch,
            onboarding_templates::render(&installation.version, &repository.default_branch)
                .map_err(|_| "setup_template_invalid")?,
        )
        .map_err(|_| "setup_template_invalid")?;
        let read_token = source
            .token(github.installation_id, &github.repository_id)
            .await?;
        let head = source
            .head(
                &github.owner,
                &github.name,
                &repository.default_branch,
                &read_token,
            )
            .await?;
        if crate::adapters::github::setup::installed_workflows_match(&request.files, |path| {
            source.file(&github.owner, &github.name, &head, path, &read_token)
        })
        .await?
        {
            let id = id.to_owned();
            let key = action.idempotency_key.clone();
            return with_store(self, move |s| {
                s.observe_setup(&id, None, v1::AutomationInstallationState::Installed, None)?;
                s.complete_external_action("InstallRepositoryAutomation", &key)
            })
            .await
            .map_err(|_| "setup_store_unavailable");
        }
        let token = source.setup_token(github).await?;
        let recorded_pull = match &installation.provider {
            Some(v1::automation_installation::Provider::Github(provider)) => {
                provider.setup_pull_request_number
            }
            None => None,
        };
        if let Some(number) = recorded_pull {
            let pull = source
                .pull(&github.owner, &github.name, number, &token)
                .await?;
            return self
                .observe_setup_pull(id, &repository, github, &request, &token, pull, action)
                .await;
        }
        let exact_pull = source
            .find_pull(
                &github.owner,
                &github.name,
                &request.branch,
                &request.base_branch,
                &token,
            )
            .await?;
        let owned_pull = if exact_pull.is_none() {
            source
                .find_setup_pull(
                    &github.owner,
                    &github.name,
                    request.title,
                    &request.base_branch,
                    &token,
                )
                .await?
        } else {
            None
        };
        let pull = if let Some(pull) = exact_pull.or(owned_pull) {
            pull
        } else {
            source.setup_branch(github, &request, &token).await?;
            source.create_pull(&github.owner,&github.name,&token,&serde_json::json!({"head":request.branch,"base":request.base_branch,"title":request.title,"body":request.body,"draft":true})).await?
        };
        self.observe_setup_pull(id, &repository, github, &request, &token, pull, action)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn observe_setup_pull(
        &self,
        id: &str,
        repository: &v1::RepositoryConnection,
        github: &v1::GithubRepository,
        request: &crate::adapters::github::setup::SetupPullRequest,
        token: &str,
        pull: serde_json::Value,
        action: &crate::adapters::sqlite::DurableAction,
    ) -> Result<(), &'static str> {
        let source = self.source.as_ref().ok_or("github_app_unconfigured")?;
        let number = pull["number"].as_u64().ok_or("setup_pull_invalid")?;
        let merged = pull["merged_at"].as_str().is_some();
        if merged {
            let head = source
                .head(
                    &github.owner,
                    &github.name,
                    &repository.default_branch,
                    token,
                )
                .await?;
            for (path, expected) in &request.files.files {
                if source
                    .file(&github.owner, &github.name, &head, path, token)
                    .await?
                    != expected.as_bytes()
                {
                    return Err("setup_workflow_mismatch");
                }
            }
        } else if pull["state"] != "open" {
            return Err("setup_pull_closed");
        }
        let id = id.to_owned();
        let key = action.idempotency_key.clone();
        with_store(self, move |s| {
            s.observe_setup(
                &id,
                Some(number),
                if merged {
                    v1::AutomationInstallationState::Installed
                } else {
                    v1::AutomationInstallationState::SetupPrOpen
                },
                None,
            )?;
            if merged {
                s.complete_external_action("InstallRepositoryAutomation", &key)?;
            }
            Ok(())
        })
        .await
        .map_err(|_| "setup_store_unavailable")
    }
}

pub(super) fn public_setup_failure(reason: &str) -> &'static str {
    match reason {
        "github_access_denied" | "github_not_found" | "github_permissions_missing" => {
            "provider_access_required"
        }
        "github_rate_limited" | "github_secondary_rate_limited" | "github_unavailable" => {
            "provider_temporarily_unavailable"
        }
        _ => "repository_automation_failed",
    }
}
