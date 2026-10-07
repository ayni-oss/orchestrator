use super::write::clear_candidate;
use super::{AppState, Status, retire_candidate, retired, secrets, v1};
use crate::registry_probe::RegistryTestReport;
impl AppState {
    pub(super) async fn reconcile_registry(&self, mut value: v1::Connection) -> Result<(), Status> {
        self.refresh_registry_state(&mut value).await?;
        if !value.enabled || (value.candidate_secret_name.is_empty() && value.status != "pending") {
            return Ok(());
        }
        let mut candidate = registry_candidate(&value);
        let namespace = std::env::var("ORCHESTRATOR_CONNECTION_NAMESPACE")
            .map_err(|_| Status::failed_precondition("connection_namespace_unconfigured"))?;
        if !self
            .recover_registry_metadata(&mut value, &mut candidate)
            .await?
        {
            return Ok(());
        }
        let cleanup_only = value.test_authentication == "passed"
            && value.test_push == "passed"
            && value.test_pull == "passed"
            && value.test_cleanup == "failed";
        match crate::adapters::ghcr::test_report(&namespace, &candidate, cleanup_only).await {
            Ok(Some((operation, report))) => {
                let completed = apply_registry_report(value, candidate, report);
                if self.persist_current_registry(completed).await? {
                    crate::adapters::ghcr::cleanup_job(&namespace, &operation)
                        .await
                        .map_err(Status::failed_precondition)?;
                }
            }
            Ok(None) => {}
            Err(reason) => {
                self.fail_registry_candidate(value, reason).await?;
            }
        }
        Ok(())
    }
    async fn refresh_registry_state(&self, value: &mut v1::Connection) -> Result<(), Status> {
        if !value.enabled || value.status != "ready" {
            return Ok(());
        }
        let adapter = secrets()?;
        let owner = owned(value, &value.secret_uid);
        let pull_owner = owned(value, &value.pull_secret_uid);
        match (
            adapter.metadata(&value.secret_name, owner).await,
            adapter.metadata(&value.pull_secret_name, pull_owner).await,
        ) {
            (Ok((uid, revision)), Ok((pull_uid, pull_revision))) => {
                let revision = format!("{uid}:{revision}:{pull_uid}:{pull_revision}");
                if value.management_source == "admin" && value.observed_revision != revision {
                    value.secret_uid = uid;
                    value.pull_secret_uid = pull_uid;
                    value.desired_revision = revision;
                    value.status = "pending".into();
                    self.persist_connection(value.clone()).await?;
                }
            }
            _ => {
                value.status = "failed".into();
                value.failure_reason = "connection_secret_unavailable".into();
                self.persist_connection(value.clone()).await?;
            }
        }
        Ok(())
    }
    async fn recover_registry_metadata(
        &self,
        value: &mut v1::Connection,
        candidate: &mut v1::Connection,
    ) -> Result<bool, Status> {
        let adapter = secrets()?;
        let push = adapter
            .metadata(
                &candidate.secret_name,
                owned(candidate, &candidate.secret_uid),
            )
            .await;
        let pull = adapter
            .metadata(
                &candidate.pull_secret_name,
                owned(candidate, &candidate.pull_secret_uid),
            )
            .await;
        if !value.candidate_secret_name.is_empty() {
            if let Ok((uid, _)) = &push {
                value.candidate_secret_uid.clone_from(uid);
            }
            if let Ok((uid, _)) = &pull {
                value.candidate_pull_secret_uid.clone_from(uid);
            }
            if !self.persist_current_registry(value.clone()).await? {
                return Ok(false);
            }
        }
        match (push, pull) {
            (Ok((uid, revision)), Ok((pull_uid, pull_revision))) => {
                candidate.observed_revision =
                    format!("{uid}:{revision}:{pull_uid}:{pull_revision}");
                candidate.secret_uid = uid;
                candidate.pull_secret_uid = pull_uid;
                Ok(true)
            }
            _ => {
                self.fail_registry_candidate(value.clone(), "connection_secret_unavailable")
                    .await?;
                Ok(false)
            }
        }
    }
    async fn persist_current_registry(&self, value: v1::Connection) -> Result<bool, Status> {
        let _guard = self.connection_lock.lock().await;
        if self.connection(value.id.clone()).await?.resource_version != value.resource_version {
            return Ok(false);
        }
        self.persist_connection(value).await?;
        Ok(true)
    }
    async fn fail_registry_candidate(
        &self,
        mut value: v1::Connection,
        reason: &str,
    ) -> Result<(), Status> {
        value.failure_reason = reason.into();
        value.status = failure_status(&value).into();
        retire_candidate(&mut value);
        self.persist_current_registry(value).await?;
        Ok(())
    }
}
fn owned<'a>(value: &'a v1::Connection, uid: &'a str) -> Option<(&'a str, &'a str)> {
    (value.management_source == "ui").then_some((value.id.as_str(), uid))
}
fn registry_candidate(value: &v1::Connection) -> v1::Connection {
    let mut candidate = value.clone();
    if !value.candidate_secret_name.is_empty() {
        candidate
            .secret_name
            .clone_from(&value.candidate_secret_name);
        candidate.secret_uid.clone_from(&value.candidate_secret_uid);
        candidate
            .pull_secret_name
            .clone_from(&value.candidate_pull_secret_name);
        candidate
            .pull_secret_uid
            .clone_from(&value.candidate_pull_secret_uid);
    }
    candidate
}
const fn failure_status(value: &v1::Connection) -> &'static str {
    if !value.candidate_secret_name.is_empty() && !value.secret_name.is_empty() {
        "ready"
    } else {
        "failed"
    }
}
fn check_result(ran: bool, passed: bool) -> String {
    if !ran {
        "not_run"
    } else if passed {
        "passed"
    } else {
        "failed"
    }
    .into()
}
fn attach_registry_report(candidate: &mut v1::Connection, report: &RegistryTestReport) {
    candidate.test_authentication = check_result(true, report.authentication);
    candidate.test_push = check_result(report.authentication, report.push);
    candidate.test_pull = check_result(report.push, report.pull);
    candidate.test_cleanup = check_result(!report.artifact_digest.is_empty(), report.cleanup);
    candidate
        .test_artifact_digest
        .clone_from(&report.artifact_digest);
    candidate
        .test_artifact_repository
        .clone_from(&report.artifact_repository);
    candidate.test_cleanup_failure_reason = if report.artifact_digest.is_empty() {
        String::new()
    } else {
        report.cleanup_failure_reason.clone()
    };
}
fn apply_registry_report(
    mut value: v1::Connection,
    mut candidate: v1::Connection,
    report: RegistryTestReport,
) -> v1::Connection {
    attach_registry_report(&mut candidate, &report);
    if report.authentication && report.push && report.pull {
        candidate.status = "ready".into();
        candidate.failure_reason.clear();
        candidate
            .desired_revision
            .clone_from(&candidate.observed_revision);
        clear_candidate(&mut candidate);
        if !value.secret_name.is_empty() && value.secret_name != candidate.secret_name {
            candidate.retired_secret_revisions.push(retired(&value));
        }
        return candidate;
    }
    value.status = failure_status(&value).into();
    value.failure_reason = report.failure_reason;
    value.test_authentication = candidate.test_authentication;
    value.test_push = candidate.test_push;
    value.test_pull = candidate.test_pull;
    value.test_cleanup = candidate.test_cleanup;
    value.test_artifact_digest = candidate.test_artifact_digest;
    value.test_cleanup_failure_reason = candidate.test_cleanup_failure_reason;
    value.test_artifact_repository = candidate.test_artifact_repository;
    retire_candidate(&mut value);
    value
}
#[cfg(test)]
mod tests {
    use super::{RegistryTestReport, apply_registry_report, v1};
    #[test]
    fn failed_authentication_keeps_unattempted_checks_distinct() {
        let value = v1::Connection {
            candidate_secret_name: "candidate".into(),
            ..Default::default()
        };
        let candidate = value.clone();
        let result = apply_registry_report(
            value,
            candidate,
            RegistryTestReport {
                failure_reason: "registry_authentication_failed".into(),
                cleanup_failure_reason: "registry_cleanup_pending".into(),
                ..Default::default()
            },
        );
        assert_eq!(result.test_authentication, "failed");
        assert_eq!(result.test_push, "not_run");
        assert_eq!(result.test_pull, "not_run");
        assert_eq!(result.test_cleanup, "not_run");
        assert!(result.test_cleanup_failure_reason.is_empty());
    }
    #[test]
    fn failed_candidate_preserves_ready_active_revision_and_records_cleanup() {
        let value = v1::Connection {
            secret_name: "active".into(),
            candidate_secret_name: "candidate".into(),
            candidate_secret_uid: "candidate-uid".into(),
            status: "pending".into(),
            ..Default::default()
        };
        let mut candidate = value.clone();
        candidate.secret_name = "candidate".into();
        let result = apply_registry_report(
            value,
            candidate,
            RegistryTestReport {
                authentication: true,
                push: true,
                pull: false,
                artifact_digest: "sha256:synthetic".into(),
                cleanup_failure_reason: "registry_cleanup_pending".into(),
                failure_reason: "registry_pull_failed".into(),
                ..Default::default()
            },
        );
        assert_eq!(result.secret_name, "active");
        assert_eq!(result.status, "ready");
        assert_eq!(result.test_pull, "failed");
        assert_eq!(result.test_cleanup, "failed");
        assert_eq!(result.retired_secret_revisions.len(), 1);
    }
}
