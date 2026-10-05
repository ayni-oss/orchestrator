use super::{Digest, ExecutionBackend, RuntimeReconciler, Sha256, assessment, v1};
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
        let mut status = v1::ReadinessStatus {
            policy: if enabled {
                "READY_AFTER_SUCCESSFUL_ASSESSMENT"
            } else {
                "DRAFT_ONLY"
            }
            .into(),
            state: "blocked".into(),
            source: "orchestrator".into(),
            authorized_head: String::new(),
            assessment_id: assessment.id.clone(),
            diagnostic: String::new(),
        };
        if self
            .observe_readiness(change, assessment, status.clone(), enabled)
            .await?
        {
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
        status.state = "pending".into();
        status.authorized_head = assessment.head_revision.clone();
        status.diagnostic = "awaiting_provider_transition".into();
        self.persist_readiness(&change.id, status.clone(), true)
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
        status.state = "completed".into();
        status.diagnostic = "automatic_readiness_confirmed".into();
        self.persist_readiness(&change.id, status, false).await
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
        mut status: v1::ReadinessStatus,
        enabled: bool,
    ) -> Result<bool, &'static str> {
        if change.state == v1::ChangeRequestState::Open as i32 {
            if let Some(previous) = &change.readiness {
                if previous.state == "completed" || previous.state == "obsolete" {
                    return Ok(true);
                }
                if previous.state == "pending" && previous.authorized_head == change.head_revision {
                    // An uncertain transition is observed, not repeated. Without
                    // a confirmed response its actor cannot be inferred safely.
                    status = previous.clone();
                    status.state = "completed".into();
                    status.source = "provider-observed".into();
                    status.diagnostic = "ready_observed_after_uncertain_response".into();
                    self.persist_readiness(&change.id, status, false).await?;
                    return Ok(true);
                }
            }
            status.state = "manual".into();
            status.source = "maintainer".into();
            status.diagnostic = "already_ready".into();
            self.persist_readiness(&change.id, status, false).await?;
            return Ok(true);
        }
        let reason = if !enabled {
            Some("draft_only_policy")
        } else if change.state != v1::ChangeRequestState::Draft as i32 {
            Some("pull_not_open_draft")
        } else if change.candidate_id.is_empty() {
            Some("human_pull_requires_manual_readiness")
        } else if assessment.head_revision != change.head_revision {
            Some("assessment_obsolete")
        } else if assessment.conclusion != v1::AssessmentConclusion::Succeeded as i32 {
            Some("assessment_not_successful")
        } else {
            None
        };
        if let Some(reason) = reason {
            // A late historical import cannot replace a current action/status.
            if assessment.head_revision != change.head_revision && change.readiness.is_some() {
                return Ok(true);
            }
            status.diagnostic = reason.into();
            self.persist_readiness(&change.id, status, false).await?;
            return Ok(true);
        }
        Ok(false)
    }

    pub(super) async fn persist_readiness(
        &self,
        change: &str,
        status: v1::ReadinessStatus,
        action: bool,
    ) -> Result<(), &'static str> {
        let change = change.to_owned();
        self.with_database(move |store| {
            store
                .save_readiness(&change, status, action)
                .map(|_| ())
                .map_err(|_| "readiness_state_rejected")
        })
        .await
    }
}
