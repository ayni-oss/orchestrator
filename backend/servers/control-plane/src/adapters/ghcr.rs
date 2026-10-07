//! GHCR connection probes use isolated Jobs; this adapter never reads Secrets.
use crate::registry_probe::{RegistryTestReport, valid_namespace};
use ayni_orchestrator_api::v1;
use ayni_orchestrator_kubernetes::registry_probe::{RegistryProbeKubernetes, RegistryProbeRequest};
use sha2::{Digest, Sha256};
use std::env;

fn operation(connection: &v1::Connection) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!(
            "{}:{}:{}",
            connection.id, connection.desired_revision, connection.resource_version
        ))
    )[..32]
        .into()
}

/// Runs or observes a frozen probe. Persist returned evidence before deleting its Job.
/// Cleanup retries use the existing digest and never upload another artifact.
///
/// # Errors
/// Returns bounded failure codes for configuration, Job failure, or timeout.
pub async fn test_report(
    namespace: &str,
    connection: &v1::Connection,
    cleanup_only: bool,
) -> Result<Option<(String, RegistryTestReport)>, &'static str> {
    if connection.kind != "ghcr"
        || !valid_namespace(&connection.image_namespace)
        || connection.secret_name.is_empty()
        || connection.pull_secret_name.is_empty()
        || connection.secret_name == connection.pull_secret_name
    {
        return Err("registry_connection_invalid");
    }
    let operation = if cleanup_only {
        connection
            .test_artifact_repository
            .strip_prefix(&format!(
                "{}/ayni-connection-test-",
                connection.image_namespace
            ))
            .filter(|id| {
                !id.is_empty() && id.len() <= 32 && id.bytes().all(|v| v.is_ascii_hexdigit())
            })
            .ok_or("registry_cleanup_identity_invalid")?
            .to_owned()
    } else {
        operation(connection)
    };
    let job_operation = if cleanup_only {
        format!("{operation}-cleanup-{:x}", connection.resource_version)
    } else {
        operation.clone()
    };
    let transport = RegistryProbeKubernetes::new(namespace)?;
    let request = RegistryProbeRequest {
        namespace: namespace.into(),
        operation: job_operation.clone(),
        artifact_operation: operation.clone(),
        image: env::var("ORCHESTRATOR_CONNECTION_TEST_IMAGE")
            .map_err(|_| "registry_test_image_missing")?,
        image_namespace: connection.image_namespace.clone(),
        push_secret: connection.secret_name.clone(),
        pull_secret: Some(connection.pull_secret_name.clone()),
        cleanup_digest: if cleanup_only {
            if connection.test_artifact_digest.is_empty() {
                return Err("registry_cleanup_artifact_missing");
            }
            Some(connection.test_artifact_digest.clone())
        } else {
            None
        },
    };
    transport.start(&request).await?;
    if let Some(report) = transport.report(&job_operation).await? {
        let mut report: RegistryTestReport =
            serde_json::from_str(&report).map_err(|_| "registry_test_result_invalid")?;
        if cleanup_only {
            report.push = connection.test_push == "passed";
            report.pull = connection.test_pull == "passed";
        }
        return Ok(Some((job_operation, report)));
    }
    Ok(None)
}

/// Deletes a test Job only after the caller has persisted its safe report.
///
/// # Errors
/// Returns a bounded code for Kubernetes deletion failure.
pub async fn cleanup_job(namespace: &str, operation: &str) -> Result<(), &'static str> {
    RegistryProbeKubernetes::new(namespace)?
        .cleanup_job(operation)
        .await
}

/// Validates a connection; callers needing detailed results should use `test_report`.
///
/// # Errors
/// Returns a bounded code for failed authentication, push, or pull. Cleanup failure
/// is evidence requiring retry and does not invalidate working credentials.
pub async fn test_connection(
    namespace: &str,
    connection: &v1::Connection,
) -> Result<(), &'static str> {
    let (_, report) = test_report(namespace, connection, false)
        .await?
        .ok_or("registry_test_pending")?;
    if report.authentication && report.push && report.pull {
        Ok(())
    } else {
        Err("registry_connection_test_failed")
    }
}
