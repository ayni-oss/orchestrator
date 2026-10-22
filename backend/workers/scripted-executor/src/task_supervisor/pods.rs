use super::{Result, Supervisor};
use ayni_orchestrator_events::task_execution::{SupervisorRequest, TaskRun, digest, worker_token};
use k8s_openapi::api::core::v1::{PersistentVolumeClaim, Pod, Secret};
use kube::{
    Api, Client,
    api::{DeleteParams, PostParams, Preconditions},
};
use serde_json::json;
use std::io;
pub async fn reconcile(s: &Supervisor, client: Client, run: &TaskRun) -> Result {
    let name = format!(
        "task-{}",
        uuid::Uuid::parse_str(&run.command.attempt_id)?.simple()
    );
    let api: Api<Pod> = Api::namespaced(client.clone(), &s.namespace);
    let pod = api.get_opt(&name).await?;
    if run.state == "cancelling" {
        if let Some(pod) = pod {
            verify(&pod, run)?;
            delete(&api, &name, &pod).await?;
            return Ok(());
        }
        s.call(&SupervisorRequest::Stopped {
            attempt_id: run.command.attempt_id.clone(),
            generation: run.command.generation,
            pod_uid: run.pod_uid.clone(),
        })
        .await?;
        return Ok(());
    }
    if let Some(pod) = pod {
        verify(&pod, run)?;
        let uid = pod.metadata.uid.clone().ok_or("task pod UID missing")?;
        if run.pod_uid.is_empty() {
            s.call(&SupervisorRequest::Observe {
                attempt_id: run.command.attempt_id.clone(),
                generation: run.generation,
                pod_uid: uid,
            })
            .await?;
            return Ok(());
        }
        if matches!(
            pod.status.as_ref().and_then(|s| s.phase.as_deref()),
            Some("Succeeded" | "Failed")
        ) {
            s.call(&SupervisorRequest::Stopped {
                attempt_id: run.command.attempt_id.clone(),
                generation: run.command.generation,
                pod_uid: run.pod_uid.clone(),
            })
            .await?;
            // The PVC remains available for inspection; no accepted changes are discarded.
        }
        return Ok(());
    }
    if !run.pod_uid.is_empty() {
        // Pod loss is interruption, never permission to resend a possibly paid call.
        s.call(&SupervisorRequest::Stopped {
            attempt_id: run.command.attempt_id.clone(),
            generation: run.command.generation,
            pod_uid: run.pod_uid.clone(),
        })
        .await?;
        return Ok(());
    }
    storage(s, client.clone(), run, &name).await?;
    let expected = pod_spec(s, run, &name)?;
    match api.create(&PostParams::default(), &expected).await {
        Ok(pod) => {
            let uid = pod.metadata.uid.ok_or("task pod UID missing")?;
            s.call(&SupervisorRequest::Observe {
                attempt_id: run.command.attempt_id.clone(),
                generation: run.generation,
                pod_uid: uid,
            })
            .await?;
        }
        Err(kube::Error::Api(e)) if e.code == 409 => {}
        Err(e) => return Err(e.into()),
    }
    Ok(())
}
fn verify(pod: &Pod, run: &TaskRun) -> Result {
    let hash = digest(&serde_json::to_vec(&run.command)?);
    let valid = pod
        .metadata
        .labels
        .as_ref()
        .is_some_and(|l| l.get("build.ayni/attempt") == Some(&run.command.attempt_id))
        && pod
            .metadata
            .annotations
            .as_ref()
            .is_some_and(|a| a.get("build.ayni/command-digest") == Some(&hash))
        && pod
            .spec
            .as_ref()
            .is_some_and(|s| s.automount_service_account_token == Some(false));
    if !valid || (!run.pod_uid.is_empty() && pod.metadata.uid.as_deref() != Some(&run.pod_uid)) {
        return Err(io::Error::other("task workload identity mismatch").into());
    }
    Ok(())
}
async fn delete(api: &Api<Pod>, name: &str, pod: &Pod) -> Result {
    let options = DeleteParams {
        preconditions: Some(Preconditions {
            uid: pod.metadata.uid.clone(),
            resource_version: pod.metadata.resource_version.clone(),
        }),
        ..Default::default()
    };
    match api.delete(name, &options).await {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(e)) if e.code == 404 => Ok(()),
        Err(e) => Err(e.into()),
    }
}
async fn storage(s: &Supervisor, client: Client, run: &TaskRun, name: &str) -> Result {
    let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), &s.namespace);
    let labels = json!({"app.kubernetes.io/name":"ayni-task-worker","build.ayni/attempt":run.command.attempt_id});
    if let Some(pvc) = pvcs.get_opt(name).await? {
        if pvc
            .metadata
            .labels
            .as_ref()
            .and_then(|l| l.get("build.ayni/attempt"))
            != Some(&run.command.attempt_id)
        {
            return Err("task storage identity mismatch".into());
        }
    } else {
        let pvc = serde_json::from_value(
            json!({"apiVersion":"v1","kind":"PersistentVolumeClaim","metadata":{"name":name,"labels":labels},"spec":{"accessModes":["ReadWriteOnce"],"resources":{"requests":{"storage":"2Gi"}}}}),
        )?;
        match pvcs.create(&PostParams::default(), &pvc).await {
            Ok(_) => {}
            Err(kube::Error::Api(e)) if e.code == 409 => {}
            Err(e) => return Err(e.into()),
        }
    }
    let secrets: Api<Secret> = Api::namespaced(client, &s.namespace);
    if let Some(secret) = secrets.get_opt(name).await? {
        if secret
            .metadata
            .labels
            .as_ref()
            .and_then(|l| l.get("build.ayni/attempt"))
            != Some(&run.command.attempt_id)
        {
            return Err("task capability identity mismatch".into());
        }
    } else {
        let token = worker_token(
            s.token.as_bytes(),
            &run.command.attempt_id,
            run.command.generation,
        );
        let secret = serde_json::from_value(
            json!({"apiVersion":"v1","kind":"Secret","metadata":{"name":name,"labels":labels},"immutable":true,"stringData":{"token":token}}),
        )?;
        match secrets.create(&PostParams::default(), &secret).await {
            Ok(_) => {}
            Err(kube::Error::Api(e)) if e.code == 409 => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
fn pod_spec(s: &Supervisor, run: &TaskRun, name: &str) -> Result<Pod> {
    let command = &run.command;
    let security = json!({"allowPrivilegeEscalation":false,"readOnlyRootFilesystem":true,"capabilities":{"drop":["ALL"]}});
    let launcher_security = json!({"allowPrivilegeEscalation":false,"readOnlyRootFilesystem":true,"runAsUser":0,"runAsGroup":10001,"runAsNonRoot":false,"capabilities":{"drop":["ALL"],"add":["SETUID","SETGID"]}});
    let env = json!([
        {"name":"ORCHESTRATOR_TASK_AUTHORITY_URL","value":s.url},{"name":"ORCHESTRATOR_TASK_ATTEMPT","value":command.attempt_id},
        {"name":"ORCHESTRATOR_TASK_GENERATION","value":command.generation.to_string()},
        {"name":"ORCHESTRATOR_TASK_TOKEN_PATH","value":"/var/run/task-capability/token"},
        {"name":"ORCHESTRATOR_TASK_POD_UID","valueFrom":{"fieldRef":{"fieldPath":"metadata.uid"}}},
        {"name":"ORCHESTRATOR_TASK_LEDGER_ROOT","value":"/var/lib/ayni-task/ledger"},
        {"name":"ORCHESTRATOR_TASK_SOURCE","value":"/workspace"},
        {"name":"ORCHESTRATOR_TASK_FIXTURE_ENABLED","value":s.fixture.to_string()},
        {"name":"ORCHESTRATOR_OPENAI_ENABLED","value":s.openai.to_string()},
        {"name":"ORCHESTRATOR_OPENAI_KEY_PATH","value":"/var/run/provider/api-key"},
        {"name":"OPENAI_BASE_URL","value":s.endpoint},{"name":"HOME","value":"/tmp/home"}
    ]);
    let runtime_env = json!([
        {"name":"HOME","value":"/tmp/home"},{"name":"XDG_CACHE_HOME","value":"/tmp/cache"},
        {"name":"MISE_CACHE_DIR","value":"/tmp/mise"},{"name":"CARGO_HOME","value":"/tmp/cargo"},
        {"name":"CARGO_TARGET_DIR","value":"/tmp/target"},{"name":"npm_config_cache","value":"/tmp/npm"},
        {"name":"GIT_CONFIG_COUNT","value":"1"},{"name":"GIT_CONFIG_KEY_0","value":"safe.directory"},{"name":"GIT_CONFIG_VALUE_0","value":"/workspace"},
        {"name":"GRADLE_USER_HOME","value":"/tmp/gradle"},{"name":"UV_CACHE_DIR","value":"/tmp/uv"}
    ]);
    let work = json!({"name":"work","mountPath":"/var/lib/ayni-task"});
    let tools_ro = json!({"name":"tools","mountPath":"/opt/ayni-task","readOnly":true});
    let capability =
        json!({"name":"capability","mountPath":"/var/run/task-capability","readOnly":true});
    let coding_temp = json!({"name":"coding-temporary","mountPath":"/tmp"});
    let frozen_ro = json!({"name":"work","mountPath":"/quality-frozen","subPath":"quality/frozen","readOnly":true});
    let private_ro = json!({"name":"work","mountPath":"/quality-private","subPath":"quality/runtime","readOnly":true});
    let source_ro =
        json!({"name":"work","mountPath":"/workspace","subPath":"quality/source","readOnly":true});
    let runtime_temp = json!({"name":"runtime-temporary","mountPath":"/tmp"});
    let artifacts =
        json!({"name":"work","mountPath":"/workspace/.ayni","subPath":"quality/artifacts"});
    let small =
        json!({"requests":{"cpu":"50m","memory":"64Mi"},"limits":{"cpu":"1","memory":"512Mi"}});
    let runtime_resources =
        json!({"requests":{"cpu":"100m","memory":"128Mi"},"limits":{"cpu":"4","memory":"8Gi"}});
    let fixture = command.provider == "scripted" && s.fixture;
    let coding_image = if fixture {
        &s.image
    } else {
        &command.runtime_image
    };
    let runtime_image = if command.runtime_image.starts_with("example.invalid/") {
        &s.image
    } else {
        &command.runtime_image
    };
    let mut initialize_env = env.clone();
    initialize_env[6]["value"] = json!("/var/lib/ayni-task/source");
    let mut coding_mounts = json!([work, {"name":"work","mountPath":"/workspace","subPath":"source"}, tools_ro, capability, coding_temp]);
    let mut volumes = json!([
        {"name":"work","persistentVolumeClaim":{"claimName":name}},{"name":"tools","emptyDir":{}},
        {"name":"coding-temporary","emptyDir":{"sizeLimit":"256Mi"}},
        {"name":"runtime-temporary","emptyDir":{"sizeLimit":"2Gi"}},
        {"name":"capability","secret":{"secretName":name}}
    ]);
    if command.provider == "openai" && s.openai {
        let secret = format!(
            "provider-openai-{}",
            uuid::Uuid::parse_str(&command.workspace_id)?.simple()
        );
        volumes.as_array_mut().ok_or("volumes")?.push(json!({"name":"provider","secret":{"secretName":secret,"items":[{"key":"api-key","path":"api-key"}]}}));
        coding_mounts
            .as_array_mut()
            .ok_or("mounts")?
            .push(json!({"name":"provider","mountPath":"/var/run/provider","readOnly":true}));
    }
    let prepare_mounts = json!([
        {"name":"work","mountPath":"/workspace","subPath":"quality/source"},
        {"name":"work","mountPath":"/workspace/.git","subPath":"quality/source/.git","readOnly":true},
        artifacts, tools_ro, frozen_ro,
        {"name":"work","mountPath":"/quality-private","subPath":"quality/runtime"},runtime_temp
    ]);
    let quality_mounts = json!([source_ro, artifacts, tools_ro, frozen_ro,
        {"name":"work","mountPath":"/quality-private","subPath":"quality/runtime"}, runtime_temp]);
    let pod = json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":name,"labels":{"app.kubernetes.io/name":"ayni-task-worker","build.ayni/attempt":command.attempt_id},"annotations":{"build.ayni/command-digest":digest(&serde_json::to_vec(command)?)}},"spec":{
        "automountServiceAccountToken":false,"restartPolicy":"OnFailure","activeDeadlineSeconds":command.max_seconds,"terminationGracePeriodSeconds":10,
        "securityContext":{"runAsNonRoot":true,"runAsUser":10001,"runAsGroup":10001,"fsGroup":10001,"seccompProfile":{"type":"RuntimeDefault"}},
        "initContainers":[
            {"name":"initialize","image":s.image,"imagePullPolicy":"IfNotPresent","command":["/usr/local/bin/ayni-task-worker","initialize"],"env":initialize_env,"securityContext":security,"volumeMounts":[work,{"name":"tools","mountPath":"/opt/ayni-task"},capability,coding_temp],"resources":small},
            {"name":"coding","image":coding_image,"imagePullPolicy":"IfNotPresent","command":["/opt/ayni-task/worker"],"env":env,"securityContext":security,"volumeMounts":coding_mounts,"resources":small},
            {"name":"freeze","image":s.image,"imagePullPolicy":"IfNotPresent","command":["/opt/ayni-task/worker","quality-freeze"],"env":env,"securityContext":security,"volumeMounts":[work,tools_ro,capability,coding_temp],"resources":small},
            {"name":"prepare","image":runtime_image,"imagePullPolicy":"IfNotPresent","command":["/opt/ayni-task/worker","quality-prepare"],"env":runtime_env,"securityContext":launcher_security,"workingDir":"/workspace","volumeMounts":prepare_mounts,"resources":runtime_resources},
            {"name":"seal","image":s.image,"imagePullPolicy":"IfNotPresent","command":["/opt/ayni-task/worker","quality-seal"],"securityContext":security,"volumeMounts":[source_ro,private_ro,tools_ro,{"name":"work","mountPath":"/quality-frozen","subPath":"quality/frozen"}],"resources":small},
            {"name":"quality","image":runtime_image,"imagePullPolicy":"IfNotPresent","command":["/opt/ayni-task/worker","quality-run"],"env":runtime_env,"securityContext":launcher_security,"workingDir":"/workspace","volumeMounts":quality_mounts,"resources":runtime_resources}
        ],
        "containers":[{"name":"collector","image":s.image,"imagePullPolicy":"IfNotPresent","command":["/opt/ayni-task/worker","quality-collect"],"env":env,"securityContext":security,"volumeMounts":[work,tools_ro,capability,frozen_ro,private_ro,coding_temp],"resources":small}],
        "volumes":volumes
    }});
    Ok(serde_json::from_value(pod)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_orchestrator_events::task_execution::TaskCommand;
    #[test]
    fn project_runtime_has_no_credentials_and_final_source_is_read_only() -> Result {
        let supervisor = Supervisor {
            http: reqwest::Client::new(),
            url: "http://authority".into(),
            token: "fixture".into(),
            namespace: "sessions".into(),
            image: "bootstrap@sha256:fixture".into(),
            fixture: false,
            openai: true,
            endpoint: "https://provider.invalid/v1".into(),
        };
        let run = TaskRun {
            command: TaskCommand {
                workspace_id: uuid::Uuid::now_v7().to_string(),
                provider: "openai".into(),
                runtime_image: "runtime@sha256:fixture".into(),
                ..Default::default()
            },
            state: "claimed".into(),
            generation: 1,
            pod_uid: String::new(),
            deadline_seconds: 1,
            provider_state: "not_started".into(),
            outcome: None,
            quality: None,
        };
        let pod = pod_spec(&supervisor, &run, "task-fixture")?;
        let spec = pod.spec.ok_or("pod spec")?;
        let init = spec.init_containers.ok_or("init stages")?;
        for stage in &init {
            let mounts = stage.volume_mounts.as_ref().ok_or("mounts")?;
            assert_eq!(
                mounts.iter().any(|m| m.name == "provider"),
                stage.name == "coding"
            );
            if matches!(stage.name.as_str(), "prepare" | "quality") {
                assert_runtime_boundary(stage)?;
            } else {
                assert_ne!(
                    stage.security_context.as_ref().and_then(|s| s.run_as_user),
                    Some(0)
                );
            }
            if stage.name == "quality" {
                assert!(
                    mounts
                        .iter()
                        .any(|m| m.mount_path == "/workspace" && m.read_only == Some(true))
                );
            }
            if stage.name == "prepare" {
                assert!(
                    mounts
                        .iter()
                        .any(|m| m.mount_path == "/workspace/.git" && m.read_only == Some(true))
                );
            }
        }
        assert!(spec.containers.iter().all(|c| {
            c.volume_mounts
                .as_ref()
                .is_some_and(|m| m.iter().all(|v| v.name != "provider"))
        }));
        assert!(spec.containers.iter().all(|c| {
            c.security_context
                .as_ref()
                .is_none_or(|s| s.run_as_user != Some(0))
        }));
        assert_eq!(
            spec.security_context.as_ref().and_then(|s| s.run_as_user),
            Some(10001)
        );
        assert_eq!(spec.automount_service_account_token, Some(false));
        Ok(())
    }
    fn assert_runtime_boundary(stage: &k8s_openapi::api::core::v1::Container) -> Result {
        let security = stage.security_context.as_ref().ok_or("runtime security")?;
        assert_eq!(security.run_as_user, Some(0));
        assert_eq!(security.run_as_group, Some(10001));
        assert_eq!(security.run_as_non_root, Some(false));
        assert_eq!(
            security
                .capabilities
                .as_ref()
                .and_then(|c| c.add.as_ref())
                .map(Vec::as_slice),
            Some(["SETUID".to_owned(), "SETGID".to_owned()].as_slice())
        );
        let mounts = stage.volume_mounts.as_ref().ok_or("runtime mounts")?;
        assert!(
            !mounts
                .iter()
                .any(|m| m.name == "capability" || m.mount_path == "/var/lib/ayni-task")
        );
        let variables = stage.env.as_ref().ok_or("runtime variables")?;
        assert!(
            variables
                .iter()
                .all(|v| !v.name.starts_with("ORCHESTRATOR_") && v.name != "OPENAI_BASE_URL")
        );
        for (name, value) in [
            ("GIT_CONFIG_COUNT", "1"),
            ("GIT_CONFIG_KEY_0", "safe.directory"),
            ("GIT_CONFIG_VALUE_0", "/workspace"),
        ] {
            assert!(
                variables
                    .iter()
                    .any(|v| v.name == name && v.value.as_deref() == Some(value))
            );
        }
        Ok(())
    }
}
