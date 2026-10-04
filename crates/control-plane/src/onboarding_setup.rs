impl AppState {
    async fn reconcile_setup_actions(&self) -> Result<(), &'static str> {
        let actions = with_store(self, |s| {
            s.pending_external_actions("InstallRepositoryAutomation")
        })
        .map_err(|_| "setup_store_unavailable")?;
        // Open PRs retain pending intent. Rotate the bounded batch so they cannot
        // starve newly requested repositories while waiting for a maintainer.
        let offset = self
            .source
            .as_ref()
            .ok_or("github_app_unconfigured")?
            .setup_action_offset(actions.len());
        for action in actions
            .iter()
            .cycle()
            .skip(offset)
            .take(actions.len().min(8))
        {
            if let Err(reason) = self.reconcile_setup_action(action).await {
                tracing::warn!(reason_code = reason, "setup action deferred");
                if let Some(id) = &action.target_id {
                    let _ = with_store(self, |s| {
                        s.observe_setup(
                            id,
                            None,
                            v1::AutomationInstallationState::Degraded,
                            Some(public_setup_failure(reason)),
                        )
                    });
                }
            }
        }
        Ok(())
    }

    async fn reconcile_setup_action(
        &self,
        action: &crate::clean_store::DurableAction,
    ) -> Result<(), &'static str> {
        let id = action
            .target_id
            .as_deref()
            .ok_or("setup_identity_missing")?;
        let (installation, profile, repository) = with_store(self, |s| {
            let installation = s.automation_installation(id)?;
            let profile = s.runtime_profile(&installation.runtime_profile_id)?;
            let repository = s.repository_connection(&profile.repository_connection_id)?;
            Ok((installation, profile, repository))
        })
        .map_err(|_| "setup_store_unavailable")?;
        let github = github(&repository).map_err(|_| "setup_provider_unsupported")?;
        let source = self.source.as_ref().ok_or("github_app_unconfigured")?;
        let request = crate::github_setup::setup_pull_request(
            &profile.id,
            &repository.default_branch,
            onboarding_templates::render(&installation.version, &repository.default_branch)
                .map_err(|_| "setup_template_invalid")?,
        )
        .map_err(|_| "setup_template_invalid")?;
        let token = source.setup_token(github).await?;
        let pull = if let Some(pull) = source
            .find_pull(
                &github.owner,
                &github.name,
                &request.branch,
                &request.base_branch,
                &token,
            )
            .await?
        {
            pull
        } else {
            source.setup_branch(github, &request, &token).await?;
            source.create_pull(&github.owner,&github.name,&token,&serde_json::json!({"head":request.branch,"base":request.base_branch,"title":request.title,"body":request.body,"draft":true})).await?
        };
        let number = pull["number"].as_u64().ok_or("setup_pull_invalid")?;
        let merged = pull["merged_at"].as_str().is_some();
        if merged {
            let head = source
                .head(
                    &github.owner,
                    &github.name,
                    &repository.default_branch,
                    &token,
                )
                .await?;
            for (path, expected) in &request.files.files {
                if source
                    .file(&github.owner, &github.name, &head, path, &token)
                    .await?
                    != expected.as_bytes()
                {
                    return Err("setup_workflow_mismatch");
                }
            }
        } else if pull["state"] != "open" {
            return Err("setup_pull_closed");
        }
        with_store(self, |s| {
            s.observe_setup(
                id,
                Some(number),
                if merged {
                    v1::AutomationInstallationState::Installed
                } else {
                    v1::AutomationInstallationState::SetupPrOpen
                },
                None,
            )?;
            if merged {
                s.complete_external_action("InstallRepositoryAutomation", &action.idempotency_key)?;
            }
            Ok(())
        })
        .map_err(|_| "setup_store_unavailable")
    }
}

fn public_setup_failure(reason: &str) -> &'static str {
    match reason {
        "github_access_denied" | "github_not_found" | "github_permissions_missing" => {
            "provider_access_required"
        }
        "github_rate_limited" | "github_unavailable" => "provider_temporarily_unavailable",
        _ => "repository_automation_failed",
    }
}
