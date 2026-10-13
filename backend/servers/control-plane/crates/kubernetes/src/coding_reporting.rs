//! Reporting credentials exist only in the trusted coding supervisor.
use ayni_orchestrator_runtime::coding_events::CodingAdmission;
use k8s_openapi::api::batch::v1::Job;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
pub fn configure(
    job: &mut Job,
    data: &mut BTreeMap<String, String>,
    admission: &CodingAdmission,
) -> Result<(), &'static str> {
    admission.configuration.validate()?;
    let mut admission = admission.clone();
    admission.binding.request_digest = format!(
        "sha256:{:x}",
        Sha256::digest(
            data.get("worker.json")
                .ok_or("coding_request_invalid")?
                .as_bytes()
        )
    );
    data.insert(
        "coding-reporting.json".into(),
        serde_json::to_string(&admission).map_err(|_| "coding_request_invalid")?,
    );
    let pod = job
        .spec
        .as_mut()
        .and_then(|s| s.template.spec.as_mut())
        .ok_or("coding_job_invalid")?;
    let supervisor = pod.containers.first_mut().ok_or("coding_job_invalid")?;
    let mounts = supervisor.volume_mounts.get_or_insert_default();
    for mount in [
        json!({"name":"request","mountPath":"/request/coding-reporting.json","subPath":"coding-reporting.json","readOnly":true}),
        json!({"name":"coding-reporting","mountPath":"/reporting","readOnly":true}),
    ] {
        mounts.push(serde_json::from_value(mount).map_err(|_| "coding_job_invalid")?);
    }
    supervisor.env.get_or_insert_default().push(
        serde_json::from_value(
            json!({"name":"POD_UID","valueFrom":{"fieldRef":{"fieldPath":"metadata.uid"}}}),
        )
        .map_err(|_| "coding_job_invalid")?,
    );
    pod.volumes.get_or_insert_default().push(serde_json::from_value(json!({"name":"coding-reporting","projected":{"defaultMode":288,"sources":[{"serviceAccountToken":{"audience":ayni_orchestrator_runtime::verification::REPORTING_AUDIENCE,"expirationSeconds":600,"path":"token"}}]}})).map_err(|_|"coding_job_invalid")?);
    Ok(())
}

impl crate::execution::RuntimeKubernetes {
    /// Validate the owned physical writer before adapting a retained terminal event.
    /// # Errors
    /// Fails closed on unavailable, ambiguous or replaced Kubernetes identities.
    pub async fn coding_reporting_identity(
        &self,
        name: &str,
    ) -> Result<Option<CodingAdmission>, &'static str> {
        use k8s_openapi::api::core::v1::{ConfigMap, Pod};
        use kube::{Api, api::ListParams};
        let jobs = Api::<Job>::namespaced(self.client.clone(), &self.configuration.namespace);
        let Some(job) = jobs
            .get_opt(name)
            .await
            .map_err(|_| "kubernetes_unavailable")?
        else {
            return Ok(None);
        };
        let maps = Api::<ConfigMap>::namespaced(self.client.clone(), &self.configuration.namespace);
        let Some(map) = maps
            .get_opt(name)
            .await
            .map_err(|_| "kubernetes_unavailable")?
        else {
            return Ok(None);
        };
        if map.immutable != Some(true) || map.metadata.name != job.metadata.name {
            return Err("coding_admission_invalid");
        }
        let Some(raw) = map
            .data
            .as_ref()
            .and_then(|d| d.get("coding-reporting.json"))
        else {
            return Ok(None);
        };
        let mut admission: CodingAdmission =
            serde_json::from_str(raw).map_err(|_| "coding_admission_invalid")?;
        let pods = Api::<Pod>::namespaced(self.client.clone(), &self.configuration.namespace)
            .list(&ListParams::default().labels(&format!("job-name={name}")))
            .await
            .map_err(|_| "kubernetes_unavailable")?;
        if pods.items.len() != 1 {
            return Err("coding_writer_ambiguous");
        }
        let pod = &pods.items[0];
        if job.metadata.uid.is_none()
            || !pod
                .metadata
                .owner_references
                .as_ref()
                .is_some_and(|owners| {
                    owners.iter().any(|owner| {
                        owner.api_version == "batch/v1"
                            && owner.kind == "Job"
                            && owner.controller == Some(true)
                            && owner.name == name
                            && Some(&owner.uid) == job.metadata.uid.as_ref()
                    })
                })
        {
            return Err("coding_writer_invalid");
        }
        admission.binding.pod_uid = pod.metadata.uid.clone().ok_or("coding_writer_invalid")?;
        Ok(Some(admission))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reporting_mount_and_pod_identity_are_supervisor_only_and_bind_exact_request()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut job: Job = serde_json::from_value(
            json!({"spec":{"template":{"spec":{"automountServiceAccountToken":false,"restartPolicy":"Never","containers":[{"name":"supervisor","image":"fixture"},{"name":"repository","image":"fixture"}]}}}}),
        )?;
        let admission = CodingAdmission {
            configuration: ayni_orchestrator_runtime::verification::ReportingConfiguration {
                source: "ayni://orchestrator/supervisors/test".into(),
                endpoint: "http://controller/events/verification".into(),
            },
            binding: ayni_orchestrator_events::v1::CodingExecutionBinding::default(),
        };
        let mut data = BTreeMap::from([("worker.json".into(), "immutable-worker-input".into())]);
        configure(&mut job, &mut data, &admission)?;
        let pod = job.spec.ok_or("spec")?.template.spec.ok_or("pod")?;
        assert_eq!(pod.automount_service_account_token, Some(false));
        let supervisor = serde_json::to_string(&pod.containers[0])?;
        assert!(supervisor.contains("/reporting") && supervisor.contains("POD_UID"));
        let repository = serde_json::to_string(&pod.containers[1])?;
        assert!(!repository.contains("/reporting") && !repository.contains("POD_UID"));
        let frozen: CodingAdmission = serde_json::from_str(&data["coding-reporting.json"])?;
        assert_eq!(
            frozen.binding.request_digest,
            format!("sha256:{:x}", Sha256::digest(b"immutable-worker-input"))
        );
        Ok(())
    }
}
