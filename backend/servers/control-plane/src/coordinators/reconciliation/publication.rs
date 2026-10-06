use super::{Digest, ExecutionBackend, RuntimeReconciler, Sha256, operation_name, v1};
use ayni_orchestrator_application::identity::GitRevision;
use ayni_orchestrator_application::publication::delivery::{
    self, Delivery, DeliveryError, ReviewState,
};
impl<E: ExecutionBackend> RuntimeReconciler<E> {
    pub(super) async fn publish_candidates(&self) -> Result<(), &'static str> {
        let actions = self
            .with_database(|store| {
                store
                    .pending_external_actions("PublishCandidate")
                    .map_err(|_| "publication_state_invalid")
            })
            .await?;
        for action in actions.into_iter().take(self.configuration.concurrency) {
            self.publish_action(action).await?;
        }
        Ok(())
    }

    async fn publish_action(
        &self,
        action: crate::adapters::sqlite::DurableAction,
    ) -> Result<(), &'static str> {
        use prost::Message;
        let request = v1::PublishCandidateRequest::decode(action.request_payload.as_slice())
            .map_err(|_| "publication_request_invalid")?;
        let (candidate, change, repository, attempt) = self
            .publication_inputs(
                &request.candidate_id,
                action
                    .target_id
                    .as_deref()
                    .ok_or("publication_identity_missing")?,
            )
            .await?;
        let profile = attempt
            .snapshot
            .as_ref()
            .and_then(|s| s.runtime.as_ref())
            .ok_or("coding_snapshot_missing")?;
        if self
            .configured_profile(&profile.runtime_profile_id)
            .await
            .is_err()
        {
            return Ok(());
        }
        if candidate.state != v1::CandidateState::Available as i32 {
            return Ok(());
        }
        let Some(v1::repository_provider::Provider::Github(github)) =
            repository.provider.and_then(|p| p.provider)
        else {
            return Err("repository_provider_unsupported");
        };
        let branch = format!("agent/candidate-{:x}", Sha256::digest(&candidate.id));
        let token = self
            .source
            .publication_token(github.installation_id, &github.repository_id)
            .await?;
        let provider_adapter = crate::adapters::github::publication::GithubPublisher {
            source: &self.source,
            repository: &github,
            token: &token,
            output: std::path::Path::new("/runtime-outputs").join(operation_name(&attempt.id, 0)),
        };
        let delivery = Delivery {
            branch,
            base_branch: request.base_branch,
            base_revision: GitRevision::parse(&candidate.base_revision)
                .map_err(|_| "publication_base_invalid")?,
            head_revision: GitRevision::parse(&change.head_revision)
                .map_err(|_| "publication_head_invalid")?,
            title: request.title,
            body: format!(
                "{}\n\n{}\n\n<!-- orchestrator-candidate:{} -->",
                request.body,
                validation_block(candidate.validation.as_ref(), &change.head_revision),
                candidate.id
            ),
        };
        let published = delivery::deliver(&provider_adapter, &delivery)
            .await
            .map_err(|error| match error {
                DeliveryError::Provider(error) => error.code,
                DeliveryError::IdentityConflict => "publication_identity_conflict",
            })?;
        let provider = v1::ChangeRequestProvider {
            provider: Some(v1::change_request_provider::Provider::Github(
                v1::GithubChangeRequest {
                    repository_id: github.repository_id,
                    pull_request_number: published.number,
                    html_url: published.url,
                },
            )),
        };
        let state = match published.state {
            ReviewState::Draft => v1::ChangeRequestState::Draft,
            ReviewState::Open => v1::ChangeRequestState::Open,
            ReviewState::Closed => v1::ChangeRequestState::Closed,
            ReviewState::Merged => v1::ChangeRequestState::Merged,
        };
        self.with_database(move |store| {
            store
                .reconcile_change_request(
                    &action.idempotency_key,
                    change.resource_version,
                    provider,
                    state,
                    &change.head_revision,
                    None,
                )
                .map_err(|_| "publication_state_changed")
        })
        .await?;
        Ok(())
    }
    pub(super) async fn publication_inputs(
        &self,
        candidate_id: &str,
        change_id: &str,
    ) -> Result<
        (
            v1::Candidate,
            v1::ChangeRequest,
            v1::RepositoryConnection,
            v1::DevelopmentAttempt,
        ),
        &'static str,
    > {
        let candidate_id = candidate_id.to_owned();
        let change_id = change_id.to_owned();
        self.with_database(move |store| {
            let candidate = store
                .candidate(&candidate_id)
                .map_err(|_| "publication_candidate_missing")?;
            let inputs = (
                candidate.clone(),
                store
                    .change_request(&change_id)
                    .map_err(|_| "publication_identity_missing")?,
                store
                    .repository_connection(&candidate.repository_connection_id)
                    .map_err(|_| "repository_missing")?,
                store
                    .development_attempt(&candidate.development_attempt_id)
                    .map_err(|_| "coding_snapshot_missing")?,
            );
            Ok(inputs)
        })
        .await
    }
}

fn validation_block(validation: Option<&v1::ValidationSummary>, head: &str) -> String {
    let marker = "<!-- orchestrator-validation:v1 -->";
    let end_marker = "<!-- /orchestrator-validation:v1 -->";
    let Some(validation) = validation else {
        return format!(
            "{marker}\n### Ayni task validation\n\nResult unavailable for `{head}`.\n{end_marker}"
        );
    };
    let outcome = v1::validation_summary::Outcome::try_from(validation.outcome)
        .ok()
        .map(|outcome| format!("{outcome:?}").to_lowercase())
        .unwrap_or_else(|| "unavailable".into());
    let mut body = format!(
        "{marker}\n### Ayni task validation: {outcome}\n\nCandidate: `{head}`\n\nTargets: {} completed, {} skipped. Warnings: {}. Failures: {}.\n",
        validation.completed_targets,
        validation.skipped_targets,
        validation.warning_offenders,
        validation.failing_offenders,
    );
    if !validation.rows.is_empty() {
        body.push_str("\n| Signal | Result |\n| --- | --- |\n");
        for row in validation.rows.iter().take(32) {
            let _ = std::fmt::Write::write_fmt(
                &mut body,
                format_args!(
                    "| {} | {} |\n",
                    row.kind,
                    if row.passed { "pass" } else { "fail" }
                ),
            );
        }
    }
    body.push_str(end_marker);
    body
}
