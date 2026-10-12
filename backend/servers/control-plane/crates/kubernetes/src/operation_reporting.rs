//! Mount retained output and reporting identity only in the trusted supervisor.
use crate::{configuration::ExecutionConfiguration, execution::RuntimeKubernetes};
use ayni_orchestrator_runtime::operation::OperationAdmission;
use k8s_openapi::api::batch::v1::Job;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

impl RuntimeKubernetes {
    pub(crate) async fn create_operation(
        &self,
        name: &str,
        mut data: BTreeMap<String, String>,
        token: &str,
        mut job: Job,
        reporting: Option<&OperationAdmission>,
        container: &str,
    ) -> Result<(), &'static str> {
        if let Some(reporting) = reporting {
            configure(
                &self.configuration,
                name,
                &mut data,
                &mut job,
                reporting,
                container,
            )?;
        }
        self.create(name, data, token, &job).await
    }
}

fn configure(
    configuration: &ExecutionConfiguration,
    name: &str,
    data: &mut BTreeMap<String, String>,
    job: &mut Job,
    reporting: &OperationAdmission,
    container: &str,
) -> Result<(), &'static str> {
    reporting.configuration.validate()?;
    let mut reporting = reporting.clone();
    // Includes all public operation inputs and the rendered authority-bearing spec.
    reporting.binding.request_digest = format!(
        "sha256:{:x}",
        Sha256::digest(
            serde_json::to_vec(&(data.clone(), &job.spec))
                .map_err(|_| "runtime_request_invalid")?
        )
    );
    data.insert(
        "operation-reporting.json".into(),
        serde_json::to_string(&reporting).map_err(|_| "runtime_request_invalid")?,
    );
    let pod = job
        .spec
        .as_mut()
        .and_then(|s| s.template.spec.as_mut())
        .ok_or("runtime_job_invalid")?;
    let supervisor = pod
        .containers
        .iter_mut()
        .find(|c| c.name == container)
        .ok_or("runtime_job_invalid")?;
    let mounts = supervisor.volume_mounts.get_or_insert_default();
    mounts.push(serde_json::from_value(json!({"name":"request","mountPath":"/request/operation-reporting.json","subPath":"operation-reporting.json","readOnly":true})).map_err(|_| "runtime_job_invalid")?);
    mounts.push(
        serde_json::from_value(
            json!({"name":"operation-output","mountPath":"/output","subPath":name}),
        )
        .map_err(|_| "runtime_job_invalid")?,
    );
    mounts.push(
        serde_json::from_value(
            json!({"name":"operation-reporting","mountPath":"/reporting","readOnly":true}),
        )
        .map_err(|_| "runtime_job_invalid")?,
    );
    supervisor.env.get_or_insert_default().push(
        serde_json::from_value(
            json!({"name":"POD_UID","valueFrom":{"fieldRef":{"fieldPath":"metadata.uid"}}}),
        )
        .map_err(|_| "runtime_job_invalid")?,
    );
    let volumes = pod.volumes.get_or_insert_default();
    if !volumes.iter().any(|v| v.name == "request") {
        volumes.push(
            serde_json::from_value(json!({"name":"request","configMap":{"name":name}}))
                .map_err(|_| "runtime_job_invalid")?,
        );
    }
    volumes.push(serde_json::from_value(json!({"name":"operation-output","persistentVolumeClaim":{"claimName":configuration.output_claim}})).map_err(|_| "runtime_job_invalid")?);
    volumes.push(serde_json::from_value(json!({"name":"operation-reporting","projected":{"defaultMode":288,"sources":[{"serviceAccountToken":{"audience":ayni_orchestrator_runtime::verification::REPORTING_AUDIENCE,"expirationSeconds":600,"path":"token"}}]}})).map_err(|_| "runtime_job_invalid")?);
    pod.init_containers.get_or_insert_default().insert(0, serde_json::from_value(json!({"name":"operation-output","image":configuration.worker_image,"args":["prepare-output",name],"securityContext":{"allowPrivilegeEscalation":false,"readOnlyRootFilesystem":true,"runAsNonRoot":true,"capabilities":{"drop":["ALL"]}},"volumeMounts":[{"name":"operation-output","mountPath":"/outputs"}]})).map_err(|_| "runtime_job_invalid")?);
    Ok(())
}
