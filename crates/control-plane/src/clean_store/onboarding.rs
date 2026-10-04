impl CleanStore {
    pub(crate) fn observe_setup(
        &mut self,
        id: &str,
        pull: Option<u64>,
        state: v1::AutomationInstallationState,
        reason: Option<&str>,
    ) -> Result<(), CleanStoreError> {
        let mut resource = self.automation_installation(id)?;
        let previous = resource.clone();
        if let Some(v1::automation_installation::Provider::Github(github)) = &mut resource.provider
            && pull.is_some()
        {
            github.setup_pull_request_number = pull;
        }
        let failure = reason.map(|reason| {
            let (message, remediation) = if reason == "github_permissions_missing" {
                (
                    "The GitHub App installation is missing a permission required to create the setup PR.",
                    "Grant the App Contents, Pull requests, and Workflows read and write access in GitHub, approve the updated permissions for this installation, then retry reconciliation.",
                )
            } else {
                (
                    reason,
                    "Review GitHub App contents, workflows and pull-request permissions; repair or reopen the owned setup PR, then retry.",
                )
            };
            v1::ResourceFailure {
                reason: v1::ErrorReason::RuntimeArtifactNotVerified as i32,
                message: message.into(),
                remediation: Some(remediation.into()),
                observed_at: None,
            }
        });
        let mut profile = self.runtime_profile(&resource.runtime_profile_id)?;
        resource.state = state as i32;
        resource.failure.clone_from(&failure);
        if resource != previous {
            let now = timestamp(unix_seconds()?);
            resource.state = state as i32;
            resource.failure = failure;
            resource.updated_at = Some(now);
            resource.resource_version += 1;
            let transaction = self
                .connection
                .transaction()
                .map_err(CleanStoreError::Database)?;
            let cursor = append_event(
                &transaction,
                PROFILE_SERVICE,
                "automation_installation",
                id,
                resource.resource_version,
                Some(&profile.repository_connection_id),
                Some(&profile.id),
                &resource,
            )?;
            transaction.execute("UPDATE automation_installations SET state=?1,payload=?2,resource_version=?3,event_cursor=?4 WHERE id=?5",params![resource.state,resource.encode_to_vec(),i64::try_from(resource.resource_version).map_err(|_| CleanStoreError::CorruptState)?,i64::try_from(cursor).map_err(|_| CleanStoreError::CorruptState)?,id]).map_err(CleanStoreError::Database)?;
            transaction.commit().map_err(CleanStoreError::Database)?;
        }
        let next = match state {
            v1::AutomationInstallationState::Installed => {
                v1::RuntimeProfileState::AwaitingEnvironment
            }
            v1::AutomationInstallationState::SetupPrOpen => v1::RuntimeProfileState::SetupPrOpen,
            _ => v1::RuntimeProfileState::Degraded,
        };
        if profile.state == v1::RuntimeProfileState::Degraded as i32
            && next == v1::RuntimeProfileState::SetupPrOpen
        {
            profile = self.transition_runtime_profile(
                &profile.id,
                profile.resource_version,
                v1::RuntimeProfileState::SetupRequired,
                None,
            )?;
        }
        if ![
            v1::RuntimeProfileState::Ready as i32,
            v1::RuntimeProfileState::Updating as i32,
        ]
        .contains(&profile.state)
            && profile.state != next as i32
        {
            profile = self.transition_runtime_profile(
                &profile.id,
                profile.resource_version,
                next,
                resource.failure,
            )?;
        }
        let _ = profile;
        Ok(())
    }

    fn schedule_requested_publications(&mut self) -> Result<(), CleanStoreError> {
        let candidates = {
            let mut statement = self.connection.prepare("SELECT c.payload FROM candidates c LEFT JOIN change_requests r ON r.candidate_id=c.id WHERE r.id IS NULL AND c.state=1 LIMIT 10001").map_err(CleanStoreError::Database)?;
            let mut rows = statement.query([]).map_err(CleanStoreError::Database)?;
            collect_bounded_payloads(&mut rows)?
        };
        for bytes in candidates {
            let candidate: v1::Candidate = decode(&bytes)?;
            let attempt = self.development_attempt(&candidate.development_attempt_id)?;
            let task = self.development_task(&attempt.development_task_id)?;
            if task.state != v1::DevelopmentTaskState::Succeeded as i32
                || task.active_attempt_id.as_deref() != Some(attempt.id.as_str())
            {
                continue;
            }
            let Some(snapshot) = task.snapshot.filter(|s| s.publish_draft) else {
                continue;
            };
            let key = format!("auto-publish:{}", candidate.id);
            self.publish_candidate(&key, &v1::PublishCandidateRequest {
                idempotency_key: key.clone(), candidate_id: candidate.id,
                title: "feat: implement requested change".into(),
                body: "Requested through the operator application. Review this draft before merging.".into(),
                base_branch: snapshot.publication_base_branch, draft: true,
            })?;
        }
        Ok(())
    }
}
