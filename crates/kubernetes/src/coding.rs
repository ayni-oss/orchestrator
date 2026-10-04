//! Shared digest-only runner wiring and durable coding result collection.
use crate::job::RuntimeJobRequest;
use crate::{
    configuration::{ExecutionConfiguration, WorkloadRuntime},
    execution::RuntimeKubernetes,
    workload,
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
        runtime: &WorkloadRuntime,
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
        runtime: &WorkloadRuntime,
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
    configuration: &ExecutionConfiguration,
    runtime: &WorkloadRuntime,
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
    let mut support = workload::workspace_support(name, &configuration.worker_image);
    manifest["metadata"]["name"] = json!(name);
    manifest["spec"]
        .as_object_mut()
        .ok_or("coding_job_invalid")?
        .remove("ttlSecondsAfterFinished");
    let pod = &mut manifest["spec"]["template"]["spec"];
    pod["serviceAccountName"] = json!(runtime.service_account);
    pod["nodeSelector"] = workload::node_selector(&runtime.platform, configuration.allow_emulation);
    pod["securityContext"]["fsGroup"] = json!(10001);
    // A trusted init mounts the output claim root only to create this attempt's
    // directory. The coding process sees only its own Kubernetes subPath.
    support.init_containers.push(json!({
        "name":"output", "image":configuration.worker_image, "args":["prepare-output",name],
        "securityContext":workload::container_security(),
        "resources":workload::container_resources(),
        "volumeMounts":[{"name":"output","mountPath":"/outputs"}]
    }));
    pod["initContainers"] = json!(support.init_containers);
    pod["containers"][0]["workingDir"] = json!("/workspace");
    let mut environment = workload::runtime_environment();
    environment.push(json!({"name":"JOB_NAME","value":name}));
    pod["containers"][0]["env"] = json!(environment);
    pod["containers"][0]["volumeMounts"] = json!([
        {"name":"workspace","mountPath":"/workspace"}, {"name":"scratch","mountPath":"/scratch"},
        {"name":"launcher","mountPath":"/launcher","readOnly":true}, {"name":"request","mountPath":"/request/coding.json","subPath":"coding.json","readOnly":true},
        {"name":"output","mountPath":"/output","subPath":name},
        {"name":"model","mountPath":"/model/api-key","subPath":"api-key","readOnly":true}
    ]);
    support.volumes.push(
        json!({"name":"output","persistentVolumeClaim":{"claimName":configuration.output_claim}}),
    );
    support.volumes.push(json!({"name":"model","secret":{"secretName":configuration.model_secret,"items":[{"key":"api-key","path":"api-key"}]}}));
    pod["volumes"] = json!(support.volumes);
    serde_json::from_value(manifest).map_err(|_| "coding_job_invalid")
}

fn build_job(
    name: &str,
    configuration: &ExecutionConfiguration,
    runtime: &WorkloadRuntime,
) -> Result<Job, &'static str> {
    let build = runtime.build.as_ref().ok_or("runtime_builder_missing")?;
    let environment = vec![
        json!({"name":"BUILD_IMAGE","value":runtime.image}),
        json!({"name":"DOCKER_HOST","value":build.engine_host}),
        json!({"name":"DOCKER_TLS_VERIFY","value":"1"}),
        json!({"name":"DOCKER_CERT_PATH","value":"/engine"}),
        json!({"name":"DOCKER_CONFIG","value":"/registry"}),
        json!({"name":"AYNI_ENV_CERTIFICATE_SIGNING_KEY","valueFrom":{"secretKeyRef":{"name":build.signing_secret,"key":"signing-key"}}}),
        json!({"name":"AYNI_ENV_CERTIFICATE_KEY_ID","valueFrom":{"secretKeyRef":{"name":build.signing_secret,"key":"key-id"}}}),
    ];
    let mut container = workload::worker_container("build", &build.image, &environment);
    let mounts = container["volumeMounts"]
        .as_array_mut()
        .ok_or("runtime_build_invalid")?;
    mounts.push(json!({"name":"engine","mountPath":"/engine","readOnly":true}));
    mounts.push(json!({"name":"registry","mountPath":"/registry","readOnly":true}));
    let mut job = workload::source_job(
        name,
        configuration,
        runtime,
        &build.service_account,
        &container,
    );
    let volumes = job["spec"]["template"]["spec"]["volumes"]
        .as_array_mut()
        .ok_or("runtime_build_invalid")?;
    volumes.push(json!({"name":"engine","secret":{"secretName":build.engine_secret}}));
    volumes.push(json!({"name":"registry","secret":{"secretName":build.registry_secret,"items":[{"key":".dockerconfigjson","path":"config.json"}]}}));
    serde_json::from_value(job).map_err(|_| "runtime_build_invalid")
}

#[cfg(test)]
mod tests;
