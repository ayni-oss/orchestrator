use super::{Result, Supervisor};
use ayni_orchestrator_events::task_execution::{SupervisorRequest, TaskRun, digest, worker_token};
use ayni_orchestrator_observability::workload;
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
    if let Some(pod) = &pod {
        let _ = arm_capture(s, client.clone(), run, &name, pod).await;
    }
    if run.state == "cancelling" {
        if let Some(pod) = pod {
            verify(&pod, run)?;
            if pod.metadata.deletion_timestamp.is_none() {
                let _ = retain_lifecycle(s, run, "cancelling", Some(&pod)).await;
            }
            delete(&api, &name, &pod).await?;
            return Ok(());
        }
        let _ = recover_capture(s, client.clone(), run, &name, None).await;
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
            retain_lifecycle(s, run, "finished", Some(&pod)).await?;
            let _ = recover_capture(s, client.clone(), run, &name, Some(&pod)).await;
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
        retain_lifecycle(s, run, "pod_lost", None).await?;
        let _ = recover_capture(s, client.clone(), run, &name, None).await;
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
async fn retain_lifecycle(
    s: &Supervisor,
    run: &TaskRun,
    event: &'static str,
    pod: Option<&Pod>,
) -> Result {
    let mut diagnostic = workload::record(
        workload::Kind::Task,
        &run.command.attempt_id,
        run.generation,
        "",
        event,
        pod,
    );
    if pod.is_none() {
        diagnostic.pod_uid.clone_from(&run.pod_uid);
    }
    s.retain_lifecycle(&diagnostic).await
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
    let labels = json!({"app.kubernetes.io/name":"ayni-task-worker","app.kubernetes.io/part-of":"ayni-orchestrator","build.ayni/attempt":run.command.attempt_id});
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
    if std::env::var_os("ORCHESTRATOR_TELEMETRY_ENDPOINT").is_some() {
        let spool_name = format!("{name}-telemetry");
        if pvcs.get_opt(&spool_name).await?.is_none() {
            let pvc = serde_json::from_value(
                json!({"apiVersion":"v1","kind":"PersistentVolumeClaim","metadata":{"name":spool_name,"labels":labels},"spec":{"accessModes":["ReadWriteOnce"],"resources":{"requests":{"storage":"1Gi"}}}}),
            )?;
            match pvcs.create(&PostParams::default(), &pvc).await {
                Ok(_) => {}
                Err(kube::Error::Api(e)) if e.code == 409 => {}
                Err(e) => return Err(e.into()),
            }
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
#[allow(
    clippy::too_many_lines,
    reason = "pod security topology remains reviewed together"
)]
fn pod_spec(s: &Supervisor, run: &TaskRun, name: &str) -> Result<Pod> {
    let command = &run.command;
    let security = json!({"allowPrivilegeEscalation":false,"readOnlyRootFilesystem":true,"capabilities":{"drop":["ALL"]}});
    let launcher_security = json!({"allowPrivilegeEscalation":false,"readOnlyRootFilesystem":true,"runAsUser":0,"runAsGroup":10001,"runAsNonRoot":false,"capabilities":{"drop":["ALL"],"add":["SETUID","SETGID","KILL"]}});
    let env = task_environment(s, command)?;
    let runtime_env = json!([
        {"name":"HOME","value":"/tmp/home"},{"name":"XDG_CACHE_HOME","value":"/tmp/cache"},
        {"name":"MISE_CACHE_DIR","value":"/tmp/mise"},{"name":"CARGO_HOME","value":"/tmp/cargo"},
        {"name":"CARGO_INCREMENTAL","value":"0"},{"name":"CARGO_TARGET_DIR","value":"/tmp/target"},{"name":"npm_config_cache","value":"/tmp/npm"},
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
        {"name":"runtime-temporary","emptyDir":{"sizeLimit":"16Gi"}},
        {"name":"capability","secret":{"secretName":name}}
    ]);
    if (command.provider == "openai" && s.openai)
        || (command.provider == "anthropic" && s.anthropic)
    {
        let secret = format!(
            "provider-{}-{}",
            command.provider,
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
    let mut pod = json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":name,"labels":{"app.kubernetes.io/name":"ayni-task-worker","app.kubernetes.io/part-of":"ayni-orchestrator","build.ayni/attempt":command.attempt_id},"annotations":{"build.ayni/command-digest":digest(&serde_json::to_vec(command)?)}},"spec":{
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
    if let Ok(endpoint) = std::env::var("ORCHESTRATOR_TELEMETRY_ENDPOINT") {
        pod["spec"]["volumes"].as_array_mut().ok_or("task volumes")?.push(json!({"name":"telemetry","persistentVolumeClaim":{"claimName":format!("{name}-telemetry")}}));
        for group in ["initContainers", "containers"] {
            for container in pod["spec"][group].as_array_mut().ok_or("task containers")? {
                let component = container["name"]
                    .as_str()
                    .ok_or("container name")?
                    .to_owned();
                if container.get("env").is_none() {
                    container["env"] = json!([]);
                }
                container["env"].as_array_mut().ok_or("task environment")?.extend([
                    json!({"name":"ORCHESTRATOR_CAPTURE_SOURCE","value":json!({"component":"task-worker","container":component,"executionId":command.attempt_id,"generation":command.generation,"stage":component}).to_string()}),
                    json!({"name":"ORCHESTRATOR_CAPTURE_POD_UID","valueFrom":{"fieldRef":{"fieldPath":"metadata.uid"}}}),
                    json!({"name":"ORCHESTRATOR_CAPTURE_POD_NAME","valueFrom":{"fieldRef":{"fieldPath":"metadata.name"}}}),
                    json!({"name":"ORCHESTRATOR_CAPTURE_NAMESPACE","valueFrom":{"fieldRef":{"fieldPath":"metadata.namespace"}}}),
                    json!({"name":"ORCHESTRATOR_TELEMETRY_SPOOL","value":"/telemetry"}),
                    json!({"name":"ORCHESTRATOR_TELEMETRY_SPOOL_BYTES","value":(std::env::var("ORCHESTRATOR_TELEMETRY_SPOOL_BYTES").ok().and_then(|v|v.parse::<u64>().ok()).unwrap_or(134_217_728)/7).max(1).to_string()}),
                    json!({"name":"ORCHESTRATOR_DEBUG","value":std::env::var("ORCHESTRATOR_DEBUG").unwrap_or_else(|_|"false".into())}),
                ]);
                // Network-isolated parents only capture files. Connected trusted stages deliver them.
                if !matches!(component.as_str(), "prepare" | "quality" | "seal") {
                    container["env"]
                        .as_array_mut()
                        .ok_or("task environment")?
                        .push(json!({"name":"ORCHESTRATOR_TELEMETRY_ENDPOINT","value":endpoint}));
                }
                container["volumeMounts"]
                    .as_array_mut()
                    .ok_or("task mounts")?
                    .push(json!({"name":"telemetry","mountPath":"/telemetry"}));
            }
        }
    }
    Ok(serde_json::from_value(pod)?)
}

fn task_environment(
    s: &Supervisor,
    command: &ayni_orchestrator_events::task_execution::TaskCommand,
) -> Result<serde_json::Value> {
    let mut env = json!([
        {"name":"ORCHESTRATOR_TASK_AUTHORITY_URL","value":s.url},{"name":"ORCHESTRATOR_TASK_ATTEMPT","value":command.attempt_id},
        {"name":"ORCHESTRATOR_TASK_GENERATION","value":command.generation.to_string()},
        {"name":"ORCHESTRATOR_TASK_TOKEN_PATH","value":"/var/run/task-capability/token"},
        {"name":"ORCHESTRATOR_TASK_POD_UID","valueFrom":{"fieldRef":{"fieldPath":"metadata.uid"}}},
        {"name":"ORCHESTRATOR_TASK_LEDGER_ROOT","value":"/var/lib/ayni-task/ledger"},
        {"name":"ORCHESTRATOR_TASK_SOURCE","value":"/workspace"},
        {"name":"ORCHESTRATOR_TASK_FIXTURE_ENABLED","value":s.fixture.to_string()},
        {"name":"ORCHESTRATOR_OPENAI_ENABLED","value":s.openai.to_string()},
        {"name":"ORCHESTRATOR_ANTHROPIC_ENABLED","value":s.anthropic.to_string()},
        {"name":"ORCHESTRATOR_ANTHROPIC_KEY_PATH","value":"/var/run/provider/api-key"},
        {"name":"ANTHROPIC_BASE_URL","value":s.anthropic_endpoint},
        {"name":"ORCHESTRATOR_OPENAI_KEY_PATH","value":"/var/run/provider/api-key"},
        {"name":"OPENAI_BASE_URL","value":s.endpoint},{"name":"HOME","value":"/tmp/home"}
    ]);
    if let Some(endpoint) = ayni_orchestrator_observability::exporter_endpoint() {
        env.as_array_mut()
            .ok_or("task environment")?
            .push(json!({"name":"OTEL_EXPORTER_OTLP_TRACES_ENDPOINT","value":endpoint}));
    }
    Ok(env)
}

async fn recover_capture(
    s: &Supervisor,
    client: Client,
    run: &TaskRun,
    name: &str,
    original: Option<&Pod>,
) -> Result {
    let Ok(endpoint) = std::env::var("ORCHESTRATOR_TELEMETRY_ENDPOINT") else {
        return Ok(());
    };
    let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), &s.namespace);
    let volume = format!("{name}-telemetry");
    let Some(pvc) = pvcs.get_opt(&volume).await? else {
        return Ok(());
    };
    if pvc
        .metadata
        .labels
        .as_ref()
        .and_then(|l| l.get("build.ayni/attempt"))
        != Some(&run.command.attempt_id)
    {
        return Err("telemetry volume identity mismatch".into());
    }
    let api: Api<Pod> = Api::namespaced(client, &s.namespace);
    let recovery = format!("{name}-logs");
    if api.get_opt(&recovery).await?.is_some() {
        return Ok(());
    }
    let pod = task_replay_spec(s, &run.command.attempt_id, name, &endpoint, original)?;
    match api.create(&PostParams::default(), &pod).await {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(e)) if e.code == 409 => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn task_replay_spec(
    s: &Supervisor,
    attempt: &str,
    name: &str,
    endpoint: &str,
    original: Option<&Pod>,
) -> Result<Pod> {
    let recovery = format!("{name}-logs");
    let volume = format!("{name}-telemetry");
    let mut pod = json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":recovery,"labels":{"app.kubernetes.io/name":"ayni-task-worker","app.kubernetes.io/part-of":"ayni-orchestrator","build.ayni/attempt":attempt,"ayni.build/telemetry-replay":"true"}},"spec":{
        "automountServiceAccountToken":false,"restartPolicy":"OnFailure","activeDeadlineSeconds":86400,
        "securityContext":{"runAsUser":10001,"runAsGroup":10001,"runAsNonRoot":true,"fsGroup":10001,"seccompProfile":{"type":"RuntimeDefault"}},
        "containers":[{"name":"replay","image":s.image,"command":["/usr/local/bin/ayni-task-worker","replay-telemetry"],
        "env":[{"name":"ORCHESTRATOR_TELEMETRY_ENDPOINT","value":endpoint},{"name":"ORCHESTRATOR_TELEMETRY_SPOOL","value":"/telemetry"},{"name":"ORCHESTRATOR_TASK_TOKEN_PATH","value":"/capability/token"}],
        "securityContext":{"allowPrivilegeEscalation":false,"readOnlyRootFilesystem":true,"capabilities":{"drop":["ALL"]}},
        "resources":{"requests":{"cpu":"10m","memory":"32Mi"},"limits":{"cpu":"250m","memory":"128Mi"}},
        "volumeMounts":[{"name":"spool","mountPath":"/telemetry"},{"name":"capability","mountPath":"/capability","readOnly":true}]}],
        "volumes":[{"name":"spool","persistentVolumeClaim":{"claimName":volume}},{"name":"capability","secret":{"secretName":name}}]}});
    if let Some(node) = original
        .and_then(|p| p.spec.as_ref())
        .and_then(|s| s.node_name.as_ref())
    {
        pod["spec"]["nodeName"] = json!(node);
    }
    Ok(serde_json::from_value(pod)?)
}
async fn arm_capture(
    s: &Supervisor,
    client: Client,
    run: &TaskRun,
    name: &str,
    pod: &Pod,
) -> Result {
    let Ok(endpoint) = std::env::var("ORCHESTRATOR_TELEMETRY_ENDPOINT") else {
        return Ok(());
    };
    let api: Api<PersistentVolumeClaim> = Api::namespaced(client, &s.namespace);
    let volume = format!("{name}-telemetry");
    let Some(pvc) = api.get_opt(&volume).await? else {
        return Ok(());
    };
    if pvc
        .metadata
        .annotations
        .as_ref()
        .is_some_and(|a| a.contains_key("ayni.build/replay-spec"))
    {
        return Ok(());
    }
    let spec = serde_json::to_string(&task_replay_spec(
        s,
        &run.command.attempt_id,
        name,
        &endpoint,
        Some(pod),
    )?)?;
    api.patch(&volume,&kube::api::PatchParams::default(),&kube::api::Patch::Merge(json!({"metadata":{"labels":{"ayni.build/telemetry-spool":"true"},"annotations":{"ayni.build/replay-spec":spec,"ayni.build/source-pod-uid":pod.metadata.uid}}}))).await?;
    Ok(())
}
async fn recover_orphans(
    s: &Supervisor,
    api: &Api<Pod>,
    pvcs: &Api<PersistentVolumeClaim>,
) -> Result {
    for pvc in pvcs
        .list(
            &kube::api::ListParams::default()
                .labels("app.kubernetes.io/name=ayni-task-worker,ayni.build/telemetry-spool=true"),
        )
        .await?
        .items
    {
        if pvc.metadata.deletion_timestamp.is_some() {
            continue;
        }
        let Some(annotations) = &pvc.metadata.annotations else {
            continue;
        };
        if annotations.contains_key("ayni.build/replay-complete") {
            continue;
        }
        let Some(spec) = annotations.get("ayni.build/replay-spec") else {
            continue;
        };
        let mut replay: Pod = serde_json::from_str(spec)?;
        let name = replay.metadata.name.clone().ok_or("replay name")?;
        let Some(original) = name.strip_suffix("-logs") else {
            continue;
        };
        if api.get_opt(&name).await?.is_some() {
            continue;
        }
        if let Some(pod) = api.get_opt(original).await? {
            if pod.metadata.uid.as_ref() != annotations.get("ayni.build/source-pod-uid")
                || !matches!(
                    pod.status.as_ref().and_then(|p| p.phase.as_deref()),
                    Some("Succeeded" | "Failed")
                )
            {
                continue;
            }
        } else if let Some(spec) = replay.spec.as_mut() {
            spec.node_name = None;
        }
        match api.create(&PostParams::default(), &replay).await {
            Ok(_) => {}
            Err(kube::Error::Api(e)) if e.code == 409 => {}
            Err(e) => return Err(e.into()),
        }
    }
    let _ = s;
    Ok(())
}

pub async fn retire_replays(s: &Supervisor, client: Client) -> Result {
    let api: Api<Pod> = Api::namespaced(client.clone(), &s.namespace);
    let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(client, &s.namespace);
    recover_orphans(s, &api, &pvcs).await?;
    for pod in api
        .list(
            &kube::api::ListParams::default()
                .labels("app.kubernetes.io/name=ayni-task-worker,ayni.build/telemetry-replay=true"),
        )
        .await?
        .items
    {
        if pod.status.as_ref().and_then(|s| s.phase.as_deref()) == Some("Succeeded")
            && let Some(name) = &pod.metadata.name
        {
            if let Some(original) = name.strip_suffix("-logs") {
                pvcs.patch(
                    &format!("{original}-telemetry"),
                    &kube::api::PatchParams::default(),
                    &kube::api::Patch::Merge(
                        json!({"metadata":{"annotations":{"ayni.build/replay-complete":"true"}}}),
                    ),
                )
                .await?;
            }
            delete(&api, name, &pod).await?;
        }
    }
    Ok(())
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
            anthropic: false,
            anthropic_endpoint: "https://api.anthropic.com/v1".into(),
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
        let runtime_scratch = spec
            .volumes
            .as_ref()
            .ok_or("volumes")?
            .iter()
            .find(|volume| volume.name == "runtime-temporary")
            .ok_or("runtime scratch")?;
        assert_eq!(
            runtime_scratch
                .empty_dir
                .as_ref()
                .and_then(|directory| directory.size_limit.as_ref())
                .map(|quantity| quantity.0.as_str()),
            Some("16Gi")
        );
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
            Some(["SETUID".to_owned(), "SETGID".to_owned(), "KILL".to_owned()].as_slice())
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
            ("CARGO_INCREMENTAL", "0"),
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
