//! Isolated, immutable project research Jobs and bounded result collection.
use crate::execution::RuntimeKubernetes;
use ayni_orchestrator_runtime::project_agent::{ProjectAgentRequest, ProjectAgentResult};
use k8s_openapi::api::{
    batch::v1::Job,
    core::v1::{ConfigMap, Pod, Secret},
};
use kube::{
    Api,
    api::{DeleteParams, LogParams, PostParams, Preconditions},
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub enum ProjectObservation {
    Missing,
    Running,
    Completed(ProjectAgentResult),
    Failed(&'static str),
}

impl RuntimeKubernetes {
    /// Create or adopt immutable inputs and an owned Job; source tokens are init-only.
    /// # Errors
    /// Returns safe codes for malformed inputs, conflicting identities or unavailable Kubernetes.
    pub async fn start_project_agent(
        &self,
        name: &str,
        request: &ProjectAgentRequest,
        source_tokens: BTreeMap<String, String>,
        model_secret: &str,
        model_uid: &str,
        model_revision: &str,
    ) -> Result<(), &'static str> {
        let serialized = serde_json::to_string(request).map_err(|_| "project_request_invalid")?;
        if serialized.len() > 900_000 {
            return Err("project_request_oversized");
        }
        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let maps: Api<ConfigMap> =
            Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let secrets: Api<Secret> =
            Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let current = secrets
            .get_metadata(model_secret)
            .await
            .map_err(|_| "project_model_credential_unavailable")?;
        if current.metadata.uid.as_deref() != Some(model_uid)
            || current.metadata.resource_version.as_deref() != Some(model_revision)
        {
            return Err("project_model_credential_changed");
        }
        let digest = format!("sha256:{:x}", Sha256::digest(serialized.as_bytes()));
        let mut job = project_job(
            name,
            &self.configuration.namespace,
            &self.configuration.worker_image,
            request.deadline_seconds,
            model_secret,
            &digest,
        )?;
        let spec_digest = format!(
            "sha256:{:x}",
            Sha256::digest(serde_json::to_vec(&job.spec).map_err(|_| "project_job_invalid")?)
        );
        job.metadata
            .annotations
            .get_or_insert_default()
            .insert("ayni.io/spec-digest".into(), spec_digest.clone());
        let map: ConfigMap = serde_json::from_value(json!({"apiVersion":"v1","kind":"ConfigMap",
            "metadata":{"name":name,"labels":{"ayni.io/runtime-operation":name}},"immutable":true,
            "data":{"project-agent.json":serialized}}))
        .map_err(|_| "project_request_invalid")?;
        match maps.create(&PostParams::default(), &map).await {
            Ok(_) => {}
            Err(kube::Error::Api(error)) if error.code == 409 => {
                let old = maps.get(name).await.map_err(|_| "kubernetes_unavailable")?;
                if !owned(&old.metadata, name) || old.data != map.data {
                    return Err("project_request_conflict");
                }
            }
            Err(_) => return Err("kubernetes_unavailable"),
        }
        let secret: Secret=serde_json::from_value(json!({"apiVersion":"v1","kind":"Secret",
            "metadata":{"name":name,"labels":{"ayni.io/runtime-operation":name}},"immutable":true,"stringData":source_tokens}))
            .map_err(|_| "project_request_invalid")?;
        match secrets.create(&PostParams::default(), &secret).await {
            Ok(_) => {}
            Err(kube::Error::Api(error)) if error.code == 409 => {
                let old = secrets
                    .get_metadata(name)
                    .await
                    .map_err(|_| "kubernetes_unavailable")?;
                if !owned(&old.metadata, name) {
                    return Err("project_source_credential_conflict");
                }
            }
            Err(_) => return Err("project_source_credential_unavailable"),
        }
        match jobs.create(&PostParams::default(), &job).await {
            Ok(_) => Ok(()),
            Err(kube::Error::Api(error)) if error.code == 409 => {
                let old = jobs.get(name).await.map_err(|_| "kubernetes_unavailable")?;
                if !owned(&old.metadata, name)
                    || !job_matches(&old, &job)
                    || old
                        .metadata
                        .annotations
                        .as_ref()
                        .and_then(|a| a.get("ayni.io/spec-digest"))
                        != Some(&spec_digest)
                {
                    return Err("project_job_conflict");
                }
                Ok(())
            }
            Err(_) => Err("project_job_creation_failed"),
        }
    }

    /// Observe the single Job-owned result Pod and collect only its bounded result marker.
    /// # Errors
    /// Returns a safe code for transport failure; invalid results are failed observations.
    pub async fn observe_project_agent(
        &self,
        name: &str,
        run_id: &str,
    ) -> Result<ProjectObservation, &'static str> {
        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let Some(job) = jobs
            .get_opt(name)
            .await
            .map_err(|_| "kubernetes_unavailable")?
        else {
            return Ok(ProjectObservation::Missing);
        };
        if !owned(&job.metadata, name) {
            return Err("project_job_conflict");
        }
        if job.status.as_ref().is_some_and(|s| {
            s.failed.unwrap_or(0) > 0
                || s.conditions
                    .as_ref()
                    .is_some_and(|cs| cs.iter().any(|c| c.type_ == "Failed" && c.status == "True"))
        }) {
            return Ok(ProjectObservation::Failed("project_agent_failed"));
        }
        if job.status.as_ref().and_then(|s| s.succeeded) != Some(1) {
            return Ok(ProjectObservation::Running);
        }
        let pod = self.owned_operation_pod(name, &job).await?;
        let status = crate::observation::container_status(&pod, "research")
            .ok_or("project_result_missing")?;
        if status
            .state
            .as_ref()
            .and_then(|s| s.terminated.as_ref())
            .is_none_or(|t| t.exit_code != 0)
        {
            if !owned(&job.metadata, name) {
                return Err("project_job_conflict");
            }
            return Ok(ProjectObservation::Failed("project_agent_failed"));
        }
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let logs = pods
            .logs(
                pod.metadata.name.as_deref().ok_or("project_pod_invalid")?,
                &LogParams {
                    container: Some("research".into()),
                    limit_bytes: Some(262_144),
                    ..Default::default()
                },
            )
            .await
            .map_err(|_| "project_result_unavailable")?;
        Ok(parse_result(&logs, run_id).map_or(
            ProjectObservation::Failed("project_result_invalid"),
            ProjectObservation::Completed,
        ))
    }

    /// Cleanup only after the caller durably accepts a result and observes stopped writers.
    /// # Errors
    /// Returns a safe code if Kubernetes is unavailable.
    pub async fn cleanup_project_agent(&self, name: &str) -> Result<(), &'static str> {
        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let maps: Api<ConfigMap> =
            Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let secrets: Api<Secret> =
            Api::namespaced(self.client.clone(), &self.configuration.namespace);
        if let Some(job) = jobs
            .get_opt(name)
            .await
            .map_err(|_| "kubernetes_unavailable")?
        {
            if job
                .status
                .as_ref()
                .is_none_or(|s| s.active.unwrap_or(0) > 0)
            {
                return Err("project_writer_not_stopped");
            }
            let pod = self.owned_operation_pod(name, &job).await?;
            if !pod
                .status
                .as_ref()
                .and_then(|s| s.phase.as_deref())
                .is_some_and(|p| matches!(p, "Succeeded" | "Failed"))
            {
                return Err("project_writer_not_stopped");
            }
            ignore_absent(
                jobs.delete(name, &delete_owned(&job.metadata, name)?)
                    .await
                    .map(|_| ()),
            )?;
        }
        if let Some(map) = maps
            .get_metadata_opt(name)
            .await
            .map_err(|_| "kubernetes_unavailable")?
        {
            ignore_absent(
                maps.delete(name, &delete_owned(&map.metadata, name)?)
                    .await
                    .map(|_| ()),
            )?;
        }
        if let Some(secret) = secrets
            .get_metadata_opt(name)
            .await
            .map_err(|_| "kubernetes_unavailable")?
        {
            ignore_absent(
                secrets
                    .delete(name, &delete_owned(&secret.metadata, name)?)
                    .await
                    .map(|_| ()),
            )?;
        }
        Ok(())
    }
}

fn owned(metadata: &kube::core::ObjectMeta, name: &str) -> bool {
    metadata
        .labels
        .as_ref()
        .and_then(|l| l.get("ayni.io/runtime-operation"))
        .is_some_and(|v| v == name)
}
fn delete_owned(
    metadata: &kube::core::ObjectMeta,
    name: &str,
) -> Result<DeleteParams, &'static str> {
    if !owned(metadata, name) {
        return Err("project_resource_conflict");
    }
    Ok(DeleteParams {
        preconditions: Some(Preconditions {
            uid: metadata.uid.clone(),
            resource_version: None,
        }),
        ..Default::default()
    })
}
fn ignore_absent(result: Result<(), kube::Error>) -> Result<(), &'static str> {
    match result {
        Ok(()) => Ok(()),
        Err(kube::Error::Api(e)) if e.code == 404 => Ok(()),
        Err(_) => Err("kubernetes_unavailable"),
    }
}
fn job_matches(actual: &Job, expected: &Job) -> bool {
    let Some((a, e)) = actual.spec.as_ref().zip(expected.spec.as_ref()) else {
        return false;
    };
    let Some((a, e)) = a.template.spec.as_ref().zip(e.template.spec.as_ref()) else {
        return false;
    };
    let containers_match =
        |a: &[k8s_openapi::api::core::v1::Container],
         e: &[k8s_openapi::api::core::v1::Container]| {
            a.len() == e.len()
                && a.iter().zip(e).all(|(a, e)| {
                    a.name == e.name
                        && a.image == e.image
                        && a.args == e.args
                        && a.command == e.command
                        && a.env == e.env
                        && a.env_from == e.env_from
                        && a.security_context == e.security_context
                        && a.volume_mounts == e.volume_mounts
                })
        };
    containers_match(&a.containers, &e.containers)
        && containers_match(
            a.init_containers.as_deref().unwrap_or_default(),
            e.init_containers.as_deref().unwrap_or_default(),
        )
        && a.automount_service_account_token == e.automount_service_account_token
        && a.security_context == e.security_context
        && a.service_account_name == e.service_account_name
        && a.host_network.unwrap_or(false) == e.host_network.unwrap_or(false)
        && a.host_pid.unwrap_or(false) == e.host_pid.unwrap_or(false)
        && a.host_ipc.unwrap_or(false) == e.host_ipc.unwrap_or(false)
        && serde_json::to_value(&actual.spec)
            .ok()
            .zip(serde_json::to_value(&expected.spec).ok())
            .is_some_and(|(a, e)| contains_expected(&a, &e))
}
fn contains_expected(actual: &serde_json::Value, expected: &serde_json::Value) -> bool {
    match expected {
        serde_json::Value::Object(e) => actual.as_object().is_some_and(|a| {
            e.iter()
                .all(|(k, v)| a.get(k).is_some_and(|x| contains_expected(x, v)))
        }),
        serde_json::Value::Array(e) => actual.as_array().is_some_and(|a| {
            a.len() == e.len() && a.iter().zip(e).all(|(a, e)| contains_expected(a, e))
        }),
        _ => actual == expected,
    }
}
fn parse_result(logs: &str, run_id: &str) -> Option<ProjectAgentResult> {
    if logs.len() > 262_144 {
        return None;
    }
    let mut results = logs
        .lines()
        .filter_map(|line| line.strip_prefix("AYNI_PROJECT_RESULT:"));
    let result: ProjectAgentResult = serde_json::from_str(results.next()?).ok()?;
    if results.next().is_some()
        || result.run_id != run_id
        || result.message.len() > 16_384
        || result.proposals.len() > 100
    {
        return None;
    }
    Some(result)
}

fn project_job(
    name: &str,
    namespace: &str,
    image: &str,
    deadline: u64,
    model_secret: &str,
    digest: &str,
) -> Result<Job, &'static str> {
    if name.len() > 63
        || name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || deadline == 0
        || deadline > 3600
    {
        return Err("project_job_invalid");
    }
    let security = json!({"allowPrivilegeEscalation":false,"readOnlyRootFilesystem":true,"capabilities":{"drop":["ALL"]}});
    serde_json::from_value(json!({"apiVersion":"batch/v1","kind":"Job","metadata":{"name":name,
        "labels":{"ayni.io/runtime-operation":name,"app.kubernetes.io/component":"project-agent"},
        "annotations":{"ayni.io/request-digest":digest}},"spec":{"backoffLimit":0,"activeDeadlineSeconds":deadline,
        "template":{"metadata":{"labels":{"ayni.io/runtime-operation":name,"app.kubernetes.io/component":"project-agent"}},
        "spec":{"restartPolicy":"Never","automountServiceAccountToken":false,"serviceAccountName":"runtime-coding",
        "affinity":crate::workload::trusted_node_affinity(),
        "securityContext":{"runAsNonRoot":true,"runAsUser":10001,"runAsGroup":10001,"fsGroup":10001,"seccompProfile":{"type":"RuntimeDefault"}},
        "initContainers":[{"name":"checkout","image":image,"args":["project-checkout"],"securityContext":security,
            "resources":{"requests":{"cpu":"100m","memory":"128Mi"},"limits":{"cpu":"1","memory":"512Mi"}},
            "volumeMounts":[{"name":"request","mountPath":"/request","readOnly":true},{"name":"source","mountPath":"/credentials","readOnly":true},
            {"name":"workspace","mountPath":"/workspace"},{"name":"scratch","mountPath":"/tmp"},{"name":"scratch","mountPath":"/scratch"}]}],
        "containers":[{"name":"research","image":image,"args":["project-agent"],"securityContext":security,
            "resources":{"requests":{"cpu":"100m","memory":"128Mi"},"limits":{"cpu":"1","memory":"1Gi"}},
            "volumeMounts":[{"name":"request","mountPath":"/request","readOnly":true},{"name":"model","mountPath":"/model","readOnly":true},
            {"name":"workspace","mountPath":"/workspace","readOnly":true},{"name":"scratch","mountPath":"/tmp"},{"name":"scratch","mountPath":"/scratch"}]}],
        "volumes":[{"name":"request","configMap":{"name":name}},{"name":"source","secret":{"secretName":name}},
            {"name":"model","secret":{"secretName":model_secret,"items":[{"key":"api-key","path":"api-key"}]}},
            {"name":"workspace","emptyDir":{"sizeLimit":"1Gi"}},{"name":"scratch","emptyDir":{"sizeLimit":"128Mi"}}]}}}}))
        .map(|mut job:Job|{job.metadata.namespace=Some(namespace.into());job}).map_err(|_|"project_job_invalid")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn adoption_rejects_changed_commands_and_credential_mounts() -> Result<(), &'static str> {
        let expected = project_job(
            "project-test",
            "test",
            "worker@sha256:abc",
            600,
            "model",
            "digest",
        )?;
        assert!(job_matches(&expected, &expected));
        let mut actual = expected.clone();
        let container = &mut actual
            .spec
            .as_mut()
            .ok_or("spec")?
            .template
            .spec
            .as_mut()
            .ok_or("pod")?
            .containers[0];
        container.command = Some(vec!["/bin/sh".into()]);
        assert!(!job_matches(&actual, &expected));
        let mut actual = expected.clone();
        let mounts = actual
            .spec
            .as_mut()
            .ok_or("spec")?
            .template
            .spec
            .as_mut()
            .ok_or("pod")?
            .containers[0]
            .volume_mounts
            .as_mut()
            .ok_or("mounts")?;
        mounts.push(k8s_openapi::api::core::v1::VolumeMount {
            name: "source".into(),
            mount_path: "/credentials".into(),
            ..Default::default()
        });
        assert!(!job_matches(&actual, &expected));
        let metadata = kube::core::ObjectMeta {
            uid: Some("frozen".into()),
            labels: Some(BTreeMap::from([(
                "ayni.io/runtime-operation".into(),
                "project-test".into(),
            )])),
            ..Default::default()
        };
        assert!(delete_owned(&metadata, "other").is_err());
        assert_eq!(
            delete_owned(&metadata, "project-test")?
                .preconditions
                .and_then(|p| p.uid)
                .as_deref(),
            Some("frozen")
        );
        Ok(())
    }
    #[test]
    fn research_cannot_write_checkout_or_mount_source_credentials() -> Result<(), &'static str> {
        let job = project_job(
            "project-abc",
            "test",
            "worker@sha256:abc",
            600,
            "model",
            "sha256:abc",
        )?;
        let pod = job.spec.ok_or("spec")?.template.spec.ok_or("pod")?;
        assert_eq!(pod.automount_service_account_token, Some(false));
        let mounts = pod.containers[0].volume_mounts.as_ref().ok_or("mounts")?;
        assert!(!mounts.iter().any(|m| m.name == "source"));
        assert_eq!(
            mounts
                .iter()
                .find(|m| m.name == "workspace")
                .and_then(|m| m.read_only),
            Some(true)
        );
        Ok(())
    }
    #[test]
    fn result_requires_exactly_one_bounded_matching_receipt() {
        let line = r#"AYNI_PROJECT_RESULT:{"run_id":"run","message":"done","proposals":[],"model_calls":1}"#;
        assert!(parse_result(line, "run").is_some());
        assert!(parse_result(line, "other").is_none());
        assert!(parse_result(&format!("{line}\n{line}"), "run").is_none());
        assert!(parse_result(&"x".repeat(262_145), "run").is_none());
    }
}
