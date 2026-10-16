use super::write::clear_candidate;
use super::{AppState, Status, retire_candidate, retired, secrets, v1};
use crate::registry_probe::RegistryTestReport;
impl AppState {
    pub(super) async fn reconcile_registry(&self, mut value: v1::Connection) -> Result<(), Status> {
        if !self.refresh_registry_state(&mut value).await? {
            return Ok(());
        }
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
        let cleanup_only = cleanup_retry(&value, &candidate);
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
    async fn refresh_registry_state(&self, value: &mut v1::Connection) -> Result<bool, Status> {
        if !refresh_registry_metadata(value) {
            return Ok(true);
        }
        let adapter = secrets()?;
        let push = adapter
            .metadata(&value.secret_name, owned(value, &value.secret_uid))
            .await;
        let pull = adapter
            .metadata(
                &value.pull_secret_name,
                owned(value, &value.pull_secret_uid),
            )
            .await;
        let previous = value.clone();
        apply_registry_metadata(value, push, pull);
        if *value != previous {
            return self.persist_current_registry(value.clone()).await;
        }
        Ok(true)
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
fn refresh_registry_metadata(value: &v1::Connection) -> bool {
    value.enabled
        && !value.secret_name.is_empty()
        && !value.pull_secret_name.is_empty()
        && (value.status == "ready"
            || (value.status == "failed"
                && value.failure_reason == "connection_secret_unavailable"))
}
fn probe_passed(value: &v1::Connection) -> bool {
    value.test_authentication == "passed"
        && value.test_push == "passed"
        && value.test_pull == "passed"
}
fn cleanup_retry(value: &v1::Connection, candidate: &v1::Connection) -> bool {
    value.candidate_secret_name.is_empty()
        && !value.observed_revision.is_empty()
        && candidate.observed_revision == value.observed_revision
        && probe_passed(value)
        && value.test_cleanup == "failed"
}
fn apply_registry_metadata(
    value: &mut v1::Connection,
    push: Result<(String, String), &'static str>,
    pull: Result<(String, String), &'static str>,
) {
    let (Ok((uid, revision)), Ok((pull_uid, pull_revision))) = (push, pull) else {
        value.status = "failed".into();
        value.failure_reason = "connection_secret_unavailable".into();
        return;
    };
    let revision = format!("{uid}:{revision}:{pull_uid}:{pull_revision}");
    let recovering = value.status == "failed";
    let changed = revision != value.observed_revision;
    if recovering || (value.management_source == "admin" && changed) {
        value.secret_uid = uid;
        value.pull_secret_uid = pull_uid;
        value.desired_revision = revision;
        // Only the previously verified credential pair can regain readiness without a new probe.
        value.status = if !changed && probe_passed(value) {
            "ready"
        } else {
            "pending"
        }
        .into();
        value.failure_reason.clear();
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
    use super::{
        RegistryTestReport, apply_registry_metadata, apply_registry_report, cleanup_retry,
        refresh_registry_metadata, v1,
    };
    fn verified() -> v1::Connection {
        v1::Connection {
            enabled: true,
            management_source: "admin".into(),
            status: "ready".into(),
            secret_name: "push".into(),
            pull_secret_name: "pull".into(),
            secret_uid: "push-uid".into(),
            pull_secret_uid: "pull-uid".into(),
            observed_revision: "push-uid:1:pull-uid:2".into(),
            desired_revision: "push-uid:1:pull-uid:2".into(),
            test_authentication: "passed".into(),
            test_push: "passed".into(),
            test_pull: "passed".into(),
            test_cleanup: "failed".into(),
            test_artifact_digest: "sha256:retained".into(),
            test_cleanup_failure_reason: "registry_cleanup_permission_required".into(),
            ..Default::default()
        }
    }
    fn metadata(push_revision: &str) -> (String, String) {
        ("push-uid".into(), push_revision.into())
    }
    fn pull_metadata() -> (String, String) {
        ("pull-uid".into(), "2".into())
    }
    #[test]
    fn restored_verified_credentials_recover_without_repeating_a_probe_or_cleanup() {
        let mut value = verified();
        apply_registry_metadata(
            &mut value,
            Err("connection_secret_unavailable"),
            Ok(pull_metadata()),
        );
        assert_eq!(value.status, "failed");
        assert!(refresh_registry_metadata(&value));
        apply_registry_metadata(&mut value, Ok(metadata("1")), Ok(pull_metadata()));
        assert_eq!(value.status, "ready");
        assert!(value.failure_reason.is_empty());
        assert_eq!(value.test_cleanup, "failed");
        assert_eq!(value.test_artifact_digest, "sha256:retained");
        assert_eq!(
            value.test_cleanup_failure_reason,
            "registry_cleanup_permission_required"
        );
    }
    #[test]
    fn changed_credentials_require_a_full_probe_even_when_prior_tests_passed() {
        for recovering in [false, true] {
            let mut value = verified();
            if recovering {
                value.status = "failed".into();
                value.failure_reason = "connection_secret_unavailable".into();
            }
            apply_registry_metadata(&mut value, Ok(metadata("3")), Ok(pull_metadata()));
            assert_eq!(value.status, "pending");
            assert_eq!(value.desired_revision, "push-uid:3:pull-uid:2");
            assert_eq!(value.observed_revision, "push-uid:1:pull-uid:2");
            let mut candidate = value.clone();
            candidate
                .observed_revision
                .clone_from(&value.desired_revision);
            assert!(!cleanup_retry(&value, &candidate));
        }
    }
    #[test]
    fn missing_pull_credentials_remain_failed_with_historical_results() {
        let mut value = verified();
        apply_registry_metadata(
            &mut value,
            Ok(metadata("1")),
            Err("connection_secret_unavailable"),
        );
        assert_eq!(value.status, "failed");
        assert_eq!(value.test_pull, "passed");
        apply_registry_metadata(
            &mut value,
            Ok(metadata("1")),
            Err("connection_secret_unavailable"),
        );
        assert_eq!(value.failure_reason, "connection_secret_unavailable");
    }
    #[test]
    fn recovery_without_successful_probe_evidence_requires_validation() {
        let mut value = verified();
        value.status = "failed".into();
        value.failure_reason = "connection_secret_unavailable".into();
        value.test_pull = "not_run".into();
        apply_registry_metadata(&mut value, Ok(metadata("1")), Ok(pull_metadata()));
        assert_eq!(value.status, "pending");
    }
    #[test]
    fn disabled_and_authentication_failed_connections_do_not_restart_automatically() {
        let mut value = verified();
        value.enabled = false;
        assert!(!refresh_registry_metadata(&value));
        value.enabled = true;
        value.status = "failed".into();
        value.failure_reason = "registry_authentication_failed".into();
        assert!(!refresh_registry_metadata(&value));
    }
    #[test]
    fn cleanup_only_reuses_evidence_only_for_the_same_credential_pair() {
        let mut value = verified();
        assert!(cleanup_retry(&value, &value));
        value.candidate_secret_name = "replacement".into();
        assert!(!cleanup_retry(&value, &value));
    }
    #[tokio::test]
    async fn stale_registry_refresh_cannot_overwrite_a_concurrent_disable()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let state = super::AppState::open(directory.path().join("state.db"), 3)?;
        let mut snapshot = verified();
        snapshot.id = "registry".into();
        snapshot.kind = "ghcr".into();
        snapshot.resource_version = 1;
        state.persist_connection(snapshot.clone()).await?;
        let mut disabled = snapshot.clone();
        disabled.enabled = false;
        disabled.status = "disabled".into();
        disabled.resource_version = 2;
        state.persist_connection(disabled).await?;
        assert!(!state.persist_current_registry(snapshot).await?);
        assert!(!state.connection("registry".into()).await?.enabled);
        state.shutdown_database().await?;
        Ok(())
    }
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
