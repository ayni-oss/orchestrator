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
    pod.security_context.get_or_insert_default().fs_group = Some(10001);
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

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_orchestrator_events::v1::{RuntimeOperationBinding, RuntimeOperationKind};
    #[test]
    fn operation_identity_and_output_mounts_are_supervisor_only()
    -> Result<(), Box<dyn std::error::Error>> {
        let configuration = ExecutionConfiguration {
            namespace: "test".into(),
            worker_image: "registry.test/worker:fixture".into(),
            output_claim: "outputs".into(),
            model_secret: "model".into(),
            allow_emulation: false,
            engine_mode: "external-isolated".into(),
        };
        let reporting = OperationAdmission {
            configuration: ayni_orchestrator_runtime::verification::ReportingConfiguration {
                source: "ayni://orchestrator/supervisors/test".into(),
                endpoint: "http://control-plane:8080/events/verification".into(),
            },
            binding: RuntimeOperationBinding {
                execution_id: "operation-one".into(),
                ..Default::default()
            },
            kind: RuntimeOperationKind::Trust,
        };
        let mut original: Job = serde_json::from_value(json!({"spec":{"template":{"spec":{
            "containers":[{"name":"supervisor","image":"fixture"},{"name":"repository","image":"fixture"}],
            "automountServiceAccountToken":false,"restartPolicy":"Never"
        }}}}))?;
        let mut job = original.clone();
        let mut data = BTreeMap::from([("execution.json".into(), "first".into())]);
        configure(
            &configuration,
            "operation-one",
            &mut data,
            &mut job,
            &reporting,
            "supervisor",
        )?;
        let admission: OperationAdmission =
            serde_json::from_str(&data["operation-reporting.json"])?;
        let pod = job.spec.ok_or("spec")?.template.spec.ok_or("pod")?;
        assert_eq!(pod.automount_service_account_token, Some(false));
        let supervisor = serde_json::to_string(&pod.containers[0])?;
        assert!(supervisor.contains("/reporting") && supervisor.contains("/output"));
        let repository = serde_json::to_string(&pod.containers[1])?;
        assert!(!repository.contains("/reporting") && !repository.contains("/output"));
        assert!(repository.contains("fixture"));
        assert_eq!(
            pod.security_context.ok_or("security")?.fs_group,
            Some(10001)
        );
        let mut second = BTreeMap::from([("execution.json".into(), "second".into())]);
        configure(
            &configuration,
            "operation-one",
            &mut second,
            &mut original,
            &reporting,
            "supervisor",
        )?;
        let changed: OperationAdmission =
            serde_json::from_str(&second["operation-reporting.json"])?;
        assert_ne!(
            admission.binding.request_digest,
            changed.binding.request_digest
        );
        assert_eq!(admission.kind, RuntimeOperationKind::Trust);
        Ok(())
    }
}
