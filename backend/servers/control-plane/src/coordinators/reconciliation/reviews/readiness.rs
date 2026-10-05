use super::{Digest, ExecutionBackend, RuntimeReconciler, Sha256, assessment, v1};
use ayni_orchestrator_application::readiness::{self, Command, Policy};
impl<E: ExecutionBackend> RuntimeReconciler<E> {
    pub(super) async fn reconcile_readiness(
        &self,
        policy: &crate::process::runtime_configuration::AssessmentConfiguration,
        github: &v1::GithubRepository,
        token: &str,
        change: &v1::ChangeRequest,
        assessment: &v1::Assessment,
        pull: &serde_json::Value,
    ) -> Result<(), &'static str> {
        use crate::process::runtime_configuration::PublicationPolicy;
        let enabled =
            policy.publication_policy == PublicationPolicy::ReadyAfterSuccessfulAssessment;
        if self.observe_readiness(change, assessment, enabled).await? {
            return Ok(());
        }
        let candidate_id = change.candidate_id.clone();
        let change_id = change.id.clone();
        let (candidate, request) = self
            .with_database(move |store| {
                Ok((
                    store
                        .candidate(&candidate_id)
                        .map_err(|_| "readiness_candidate_missing")?,
                    store
                        .publication_request_for(&change_id)
                        .map_err(|_| "readiness_publication_missing")?,
                ))
            })
            .await?;
        let candidate_head = candidate
            .identity
            .as_ref()
            .and_then(|i| i.identity.as_ref())
            .and_then(|i| match i {
                v1::candidate_identity::Identity::Git(g) => Some(g.commit_sha.as_str()),
                v1::candidate_identity::Identity::Bundle(_) => None,
            })
            .ok_or("readiness_candidate_invalid")?;
        if candidate_head != assessment.head_revision
            || pull["base"]["ref"] != request.base_branch
            || pull["head"]["repo"]["id"] != pull["base"]["repo"]["id"]
            || pull["head"]["ref"] != format!("agent/candidate-{:x}", Sha256::digest(&candidate.id))
        {
            return Err("readiness_ownership_mismatch");
        }
        let command = Command {
            change_id: change.id.clone(),
            assessment_id: assessment.id.clone(),
            policy: Policy::AfterSuccessfulAssessment,
        };
        self.with_database(move |store| {
            readiness::prepare(store, &command).map_err(|_| "readiness_state_rejected")
        })
        .await?;
        self.verify_ready_check(github, token, assessment).await?;
        let number = pull["number"].as_u64().ok_or("readiness_pull_invalid")?;
        let current = self
            .source
            .review_get(github, &format!("pulls/{number}"), token)
            .await?;
        if current["head"]["sha"] != assessment.head_revision
            || current["base"]["ref"] != request.base_branch
            || current["state"] != "open"
            || current["draft"] != true
        {
            return Err("readiness_provider_state_changed");
        }
        let evidence = assessment.clone();
        self.with_database(move |store| {
            store
                .verify_assessment_files(&evidence)
                .map_err(|_| "readiness_evidence_corrupt")
        })
        .await?;
        let node = current["node_id"]
            .as_str()
            .ok_or("readiness_pull_invalid")?;
        let action = crate::adapters::sqlite::readiness_key(&change.id, &assessment.id);
        self.source.mark_ready(node, &action, token).await?;
        let command = Command {
            change_id: change.id.clone(),
            assessment_id: assessment.id.clone(),
            policy: Policy::AfterSuccessfulAssessment,
        };
        self.with_database(move |store| {
            readiness::confirm(store, &command).map_err(|_| "readiness_state_rejected")
        })
        .await
    }

    pub(super) async fn verify_ready_check(
        &self,
        github: &v1::GithubRepository,
        token: &str,
        assessment: &v1::Assessment,
    ) -> Result<(), &'static str> {
        let Some(v1::workflow_execution::Provider::Github(workflow)) = assessment
            .workflow
            .as_ref()
            .and_then(|w| w.provider.as_ref())
        else {
            return Err("readiness_workflow_missing");
        };
        let external = format!("assessment:{}:{}", workflow.run_id, workflow.attempt);
        let checks = self
            .source
            .review_get(
                github,
                &format!(
                    "commits/{}/check-runs?per_page=100",
                    assessment.head_revision
                ),
                token,
            )
            .await?;
        let checks = checks["check_runs"]
            .as_array()
            .filter(|c| c.len() < 100)
            .ok_or("readiness_checks_invalid")?;
        let latest = checks
            .iter()
            .filter(|c| c["name"] == assessment::CHECK_NAME && c["app"]["slug"] == "github-actions")
            .max_by_key(|c| c["id"].as_u64().unwrap_or(0))
            .ok_or("readiness_check_missing")?;
        if latest["external_id"] != external
            || latest["conclusion"] != "success"
            || latest["status"] != "completed"
            || latest["head_sha"] != assessment.head_revision
            || latest["details_url"] != workflow.html_url
        {
            return Err("readiness_check_not_successful");
        }
        Ok(())
    }

    pub(super) async fn observe_readiness(
        &self,
        change: &v1::ChangeRequest,
        assessment: &v1::Assessment,
        enabled: bool,
    ) -> Result<bool, &'static str> {
        use ayni_orchestrator_application::readiness::{self, Command, Outcome, Policy};
        let command = Command {
            change_id: change.id.clone(),
            assessment_id: assessment.id.clone(),
            policy: if enabled {
                Policy::AfterSuccessfulAssessment
            } else {
                Policy::DraftOnly
            },
        };
        self.with_database(move |store| {
            readiness::reconcile(store, &command)
                .map(|outcome| outcome != Outcome::RequiresVerification)
                .map_err(|_| "readiness_state_rejected")
        })
        .await
    }
}
