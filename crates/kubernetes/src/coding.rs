//! Shared digest-only runner wiring and durable coding result collection.
use crate::job::RuntimeJobRequest;
use crate::{
    configuration::{ProfileRuntime, RuntimeConfiguration},
    execution::{RuntimeKubernetes, verification_job},
};
use ayni_orchestrator_profile_ayni::{coding::CodingRequest, runtime_execution::ExecutionRequest};
use ayni_orchestrator_runtime::runtime_contract::{
    LauncherArguments, RuntimeContract, RuntimeImage, RuntimePlatform, RuntimeUser,
};
use k8s_openapi::api::batch::v1::Job;
use kube::Api;
use serde_json::json;

impl RuntimeKubernetes {
    /// Starts or adopts a dedicated build Job using operator-scoped authority.
    ///
    /// # Errors
    ///
    /// Returns a bounded error code for missing build configuration, invalid
    /// requests, Kubernetes failures, or a conflicting operation identity.
    pub async fn start_build(
        &self,
        name: &str,
        runtime: &ProfileRuntime,
        request: &ExecutionRequest,
        token: &str,
    ) -> Result<(), &'static str> {
        let job = build_job(name, &self.configuration, runtime)?;
        self.create(
            name,
            std::collections::BTreeMap::from([(
                "execution.json".into(),
                serde_json::to_string(request).map_err(|_| "runtime_request_invalid")?,
            )]),
            token,
            &job,
        )
        .await
    }

    /// Starts or adopts a coding Job using the attempt's verified image digest.
    ///
    /// # Errors
    ///
    /// Returns a bounded error code for invalid runtime admission or request
    /// data, Kubernetes failures, or a conflicting operation identity.
    pub async fn start_coding(
        &self,
        name: &str,
        runtime: &ProfileRuntime,
        request: &CodingRequest,
        token: &str,
        image: &str,
        deadline: u32,
    ) -> Result<(), &'static str> {
        let job = coding_job(name, &self.configuration, runtime, image, deadline)?;
        let data = std::collections::BTreeMap::from([
            (
                "execution.json".into(),
                serde_json::to_string(&request.execution).map_err(|_| "coding_request_invalid")?,
            ),
            (
                "coding.json".into(),
                serde_json::to_string(request).map_err(|_| "coding_request_invalid")?,
            ),
        ]);
        self.create(name, data, token, &job).await
    }

    /// Reports the current completion state of a coding Job.
    ///
    /// # Errors
    ///
    /// Returns a bounded error code when the Kubernetes Job cannot be inspected.
    pub async fn coding_state(&self, name: &str) -> Result<&'static str, &'static str> {
        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.configuration.namespace);
        let Some(job) = jobs
            .get_opt(name)
            .await
            .map_err(|_| "kubernetes_unavailable")?
        else {
            return Ok("missing");
        };
        let status = job.status.as_ref();
        if status.and_then(|s| s.succeeded).unwrap_or(0) == 1 {
            Ok("succeeded")
        } else if status.and_then(|s| s.failed).unwrap_or(0) > 0
            || status
                .and_then(|s| s.conditions.as_ref())
                .is_some_and(|cs| cs.iter().any(|c| c.type_ == "Failed" && c.status == "True"))
        {
            Ok("failed")
        } else {
            Ok("running")
        }
    }
}

fn coding_job(
    name: &str,
    configuration: &RuntimeConfiguration,
    runtime: &ProfileRuntime,
    image: &str,
    deadline: u32,
) -> Result<Job, &'static str> {
    let contract = RuntimeContract::new(
        RuntimeImage::parse(image).map_err(|_| "coding_digest_invalid")?,
        RuntimePlatform::parse(&runtime.platform).map_err(|_| "coding_platform_invalid")?,
        LauncherArguments::new(vec!["/launcher/runtime-worker".into(), "code".into()])
            .map_err(|_| "coding_launcher_invalid")?,
        RuntimeUser::new(10001, 10001).map_err(|_| "coding_user_invalid")?,
    );
    let shared = RuntimeJobRequest {
        attempt_id: name.into(),
        namespace: configuration.namespace.clone(),
        runtime: contract,
        pull_secret: runtime.pull_secret.clone(),
        launcher_config_map: name.into(),
        deadline_seconds: deadline,
    }
    .render()
    .map_err(|_| "coding_job_invalid")?;
    let mut manifest = shared.manifest;
    // Preserve the shared runner's container launch and isolation. Attach only
    // the required trusted checkout/launcher init support and attempt output.
    let support = serde_json::to_value(verification_job(name, configuration, runtime)?)
        .map_err(|_| "coding_job_invalid")?;
    manifest["metadata"]["name"] = json!(name);
    manifest["spec"]
        .as_object_mut()
        .ok_or("coding_job_invalid")?
        .remove("ttlSecondsAfterFinished");
    let pod = &mut manifest["spec"]["template"]["spec"];
    pod["serviceAccountName"] = json!(runtime.service_account);
    pod["nodeSelector"] = support["spec"]["template"]["spec"]["nodeSelector"].clone();
    pod["securityContext"]["fsGroup"] = json!(10001);
    pod["initContainers"] = support["spec"]["template"]["spec"]["initContainers"].clone();
    // A trusted init mounts the output claim root only to create this attempt's
    // directory. The coding process sees only its own Kubernetes subPath.
    pod["initContainers"].as_array_mut().ok_or("coding_job_invalid")?.push(json!({
        "name":"output", "image":configuration.worker_image, "args":["prepare-output",name],
        "securityContext":support["spec"]["template"]["spec"]["initContainers"][0]["securityContext"],
        "resources":support["spec"]["template"]["spec"]["initContainers"][0]["resources"],
        "volumeMounts":[{"name":"output","mountPath":"/outputs"}]
    }));
    pod["containers"][0]["workingDir"] = json!("/workspace");
    pod["containers"][0]["env"] = crate::execution::runtime_environment();
    pod["containers"][0]["env"]
        .as_array_mut()
        .ok_or("coding_job_invalid")?
        .push(json!({"name":"JOB_NAME","value":name}));
    pod["containers"][0]["volumeMounts"] = json!([
        {"name":"workspace","mountPath":"/workspace"}, {"name":"scratch","mountPath":"/scratch"},
        {"name":"launcher","mountPath":"/launcher","readOnly":true}, {"name":"request","mountPath":"/request/coding.json","subPath":"coding.json","readOnly":true},
        {"name":"output","mountPath":"/output","subPath":name},
        {"name":"model","mountPath":"/model/api-key","subPath":"api-key","readOnly":true}
    ]);
    pod["volumes"] = support["spec"]["template"]["spec"]["volumes"].clone();
    let volumes = pod["volumes"].as_array_mut().ok_or("coding_job_invalid")?;
    volumes.push(
        json!({"name":"output","persistentVolumeClaim":{"claimName":configuration.output_claim}}),
    );
    volumes.push(json!({"name":"model","secret":{"secretName":configuration.model_secret,"items":[{"key":"api-key","path":"api-key"}]}}));
    serde_json::from_value(manifest).map_err(|_| "coding_job_invalid")
}

fn build_job(
    name: &str,
    configuration: &RuntimeConfiguration,
    runtime: &ProfileRuntime,
) -> Result<Job, &'static str> {
    let build = runtime.build.as_ref().ok_or("runtime_builder_missing")?;
    let mut job = serde_json::to_value(verification_job(name, configuration, runtime)?)
        .map_err(|_| "runtime_build_invalid")?;
    let pod = &mut job["spec"]["template"]["spec"];
    pod["serviceAccountName"] = json!(build.service_account);
    let container = &mut pod["containers"][0];
    container["name"] = json!("build");
    container["image"] = json!(build.image);
    container["args"] = json!(["/launcher/runtime-worker", "build"]);
    container["env"] = json!([
        {"name":"BUILD_IMAGE","value":runtime.image}, {"name":"DOCKER_HOST","value":build.engine_host},
        {"name":"DOCKER_TLS_VERIFY","value":"1"}, {"name":"DOCKER_CERT_PATH","value":"/engine"}, {"name":"DOCKER_CONFIG","value":"/registry"},
        {"name":"AYNI_ENV_CERTIFICATE_SIGNING_KEY","valueFrom":{"secretKeyRef":{"name":build.signing_secret,"key":"signing-key"}}},
        {"name":"AYNI_ENV_CERTIFICATE_KEY_ID","valueFrom":{"secretKeyRef":{"name":build.signing_secret,"key":"key-id"}}}
    ]);
    let mounts = container["volumeMounts"]
        .as_array_mut()
        .ok_or("runtime_build_invalid")?;
    mounts.push(json!({"name":"engine","mountPath":"/engine","readOnly":true}));
    mounts.push(json!({"name":"registry","mountPath":"/registry","readOnly":true}));
    let volumes = pod["volumes"]
        .as_array_mut()
        .ok_or("runtime_build_invalid")?;
    volumes.push(json!({"name":"engine","secret":{"secretName":build.engine_secret}}));
    volumes.push(json!({"name":"registry","secret":{"secretName":build.registry_secret,"items":[{"key":".dockerconfigjson","path":"config.json"}]}}));
    serde_json::from_value(job).map_err(|_| "runtime_build_invalid")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Result<(RuntimeConfiguration, ProfileRuntime), serde_json::Error> {
        Ok((
            serde_json::from_value(json!({
                "namespace":"runtime-test", "worker_image":format!("registry.test/worker@sha256:{}", "a".repeat(64)),
                "output_claim":"outputs", "model_secret":"model", "profiles":{}
            }))?,
            serde_json::from_value(json!({
                "image":"registry.test/team/runtime:lock", "platform":"linux/amd64", "pull_secret":"pull-only",
                "build":{"image":format!("registry.test/builder@sha256:{}", "b".repeat(64)), "engine_host":"tcp://engine.test:2376", "engine_secret":"engine-only", "registry_secret":"push-only", "signing_secret":"signing-only"}
            }))?,
        ))
    }

    #[test]
    fn coding_uses_shared_digest_launch_without_build_authority()
    -> Result<(), Box<dyn std::error::Error>> {
        let (configuration, runtime) = fixture()?;
        let image = format!("registry.test/team/runtime@sha256:{}", "c".repeat(64));
        let job = coding_job("coding-test", &configuration, &runtime, &image, 3600)?;
        let pod = job.spec.ok_or("spec")?.template.spec.ok_or("pod")?;
        assert!(
            pod.containers[0]
                .volume_mounts
                .iter()
                .flatten()
                .any(|m| m.name == "request"
                    && m.mount_path == "/request/coding.json"
                    && m.sub_path.as_deref() == Some("coding.json")
                    && m.read_only == Some(true))
        );
        assert!(
            pod.containers[0]
                .volume_mounts
                .iter()
                .flatten()
                .any(|m| m.name == "model"
                    && m.mount_path == "/model/api-key"
                    && m.sub_path.as_deref() == Some("api-key")
                    && m.read_only == Some(true))
        );
        assert_eq!(pod.containers.len(), 1);
        assert_eq!(pod.containers[0].image.as_deref(), Some(image.as_str()));
        assert!(pod.containers[0].command.is_none());
        assert!(pod.node_name.is_none());
        assert_eq!(pod.service_account_name.as_deref(), Some("runtime-coding"));
        assert_eq!(pod.automount_service_account_token, Some(false));
        let json = serde_json::to_string(&pod)?;
        for forbidden in [
            "push-only",
            "signing-only",
            "engine-only",
            "hostPath",
            "privileged",
            "DOCKER_HOST",
        ] {
            assert!(!json.contains(forbidden));
        }
        assert!(json.contains("\"subPath\":\"coding-test\""));
        assert!(
            coding_job(
                "coding-test",
                &configuration,
                &runtime,
                "registry.test/runtime:mutable",
                3600
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn build_has_separate_service_account_and_secret_references_only()
    -> Result<(), Box<dyn std::error::Error>> {
        let (configuration, runtime) = fixture()?;
        let job = build_job("build-test", &configuration, &runtime)?;
        let spec = job.spec.ok_or("spec")?;
        assert_eq!(spec.backoff_limit, Some(0));
        assert_eq!(spec.active_deadline_seconds, Some(1800));
        let pod = spec.template.spec.ok_or("pod")?;
        assert_eq!(pod.service_account_name.as_deref(), Some("runtime-builder"));
        assert_eq!(pod.automount_service_account_token, Some(false));
        for container in pod
            .containers
            .iter()
            .chain(pod.init_containers.as_ref().ok_or("init")?)
        {
            let security = container.security_context.as_ref().ok_or("security")?;
            assert_eq!(security.allow_privilege_escalation, Some(false));
            assert_eq!(security.read_only_root_filesystem, Some(true));
            assert_ne!(security.privileged, Some(true));
        }
        assert!(
            pod.volumes
                .as_ref()
                .ok_or("volumes")?
                .iter()
                .all(|v| v.host_path.is_none() && v.persistent_volume_claim.is_none())
        );
        let json = serde_json::to_string(&pod)?;
        for required in [
            "DOCKER_TLS_VERIFY",
            "push-only",
            "signing-only",
            "engine-only",
        ] {
            assert!(json.contains(required));
        }
        assert!(!json.contains("model"));
        Ok(())
    }
}
