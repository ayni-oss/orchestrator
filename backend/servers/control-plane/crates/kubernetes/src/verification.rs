//! Retained evidence is read before interpreting Kubernetes completion.
use crate::{
    execution::{Observation, RuntimeKubernetes},
    observation,
};
use ayni_orchestrator_runtime::{
    runtime_execution::read_bounded,
    verification::{
        CONTRACT, MAX_RECORD_BYTES, RECORD_FILE, VerificationOutcome, VerificationRecord,
        valid_execution,
    },
};
use k8s_openapi::api::{
    batch::v1::Job,
    core::v1::{ConfigMap, ContainerStatus},
};
use kube::Api;
use sha2::{Digest, Sha256};
use std::path::Path;

impl RuntimeKubernetes {
    pub(crate) async fn observe_retained(
        &self,
        name: &str,
        job: &Job,
    ) -> Result<Observation, &'static str> {
        if !valid_execution(name) {
            return Err("runtime_request_invalid");
        }
        let path = Path::new("/runtime-outputs").join(name).join(RECORD_FILE);
        let bytes = match read_bounded(&path, MAX_RECORD_BYTES) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let terminal = job.status.as_ref().is_some_and(|s| {
                    s.succeeded.unwrap_or(0) > 0
                        || s.failed.unwrap_or(0) > 0
                        || s.conditions.as_ref().is_some_and(|cs| {
                            cs.iter().any(|c| {
                                c.status == "True"
                                    && matches!(c.type_.as_str(), "Failed" | "Complete")
                            })
                        })
                });
                if !terminal {
                    return Ok(Observation::Running);
                }
                let message = self.operation_message(name, "verify").await?;
                return Ok(Observation::Failed(match message.as_str() {
                    "runtime_reporting_failed" => "runtime_reporting_failed",
                    "runtime_request_invalid" => "runtime_request_invalid",
                    _ => "runtime_report_missing",
                }));
            }
            Err(_) => return Err("runtime_report_unavailable"),
        };
        let pod = self.owned_operation_pod(name, job).await?;
        let maps: Api<ConfigMap> =
            Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let request = maps
            .get(name)
            .await
            .map_err(|_| "kubernetes_unavailable")?
            .data
            .and_then(|mut data| data.remove("execution.json"))
            .ok_or("runtime_request_invalid")?;
        let status =
            observation::container_status(&pod, "verify").ok_or("runtime_result_missing")?;
        decode(
            &bytes,
            name,
            pod.metadata
                .uid
                .as_deref()
                .ok_or("runtime_pod_identity_invalid")?,
            &request,
            status,
        )
    }
}

fn decode(
    bytes: &[u8],
    name: &str,
    pod_uid: &str,
    request: &str,
    status: &ContainerStatus,
) -> Result<Observation, &'static str> {
    if bytes.len() > MAX_RECORD_BYTES {
        return Err("runtime_report_oversized");
    }
    let record: VerificationRecord<serde_json::Value> =
        serde_json::from_slice(bytes).map_err(|_| "runtime_result_invalid")?;
    if record.version != CONTRACT
        || record.execution != name
        || record.pod_uid != pod_uid
        || record.request_digest != format!("sha256:{:x}", Sha256::digest(request.as_bytes()))
    {
        return Err("runtime_report_identity_invalid");
    }
    match record.outcome {
        VerificationOutcome::Failed { reason } => Ok(Observation::Failed(reason.code())),
        VerificationOutcome::Report { report } => {
            let digest = status
                .image_id
                .rsplit_once("@sha256:")
                .map(|(_, digest)| format!("sha256:{digest}"))
                .filter(|value| {
                    value.len() == 71
                        && value[7..]
                            .bytes()
                            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
                })
                .ok_or("runtime_digest_unavailable")?;
            Ok(Observation::Completed {
                digest,
                report: serde_json::to_string(&report).map_err(|_| "runtime_result_invalid")?,
            })
        }
    }
}

#[cfg(test)]
mod tests;
