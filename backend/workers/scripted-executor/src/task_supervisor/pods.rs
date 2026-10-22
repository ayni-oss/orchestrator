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
    {"name":"OPENAI_BASE_URL","value":s.endpoint},
    {"name":"HOME","value":"/tmp/home"}
    ]);
    let mounts = json!([{"name":"work","mountPath":"/var/lib/ayni-task"},{"name":"work","mountPath":"/workspace","subPath":"source"},{"name":"tools","mountPath":"/opt/ayni-task","readOnly":true},{"name":"capability","mountPath":"/var/run/task-capability","readOnly":true},{"name":"temporary","mountPath":"/tmp"}]);
    let fixture = command.provider == "scripted" && s.fixture;
    let image = if fixture {
        &s.image
    } else {
        &command.runtime_image
    };
    let mut pod = json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":name,"labels":{"app.kubernetes.io/name":"ayni-task-worker","build.ayni/attempt":command.attempt_id},"annotations":{"build.ayni/command-digest":digest(&serde_json::to_vec(command)?)}},"spec":{"automountServiceAccountToken":false,"restartPolicy":"OnFailure","activeDeadlineSeconds":command.max_seconds,"terminationGracePeriodSeconds":10,"securityContext":{"runAsNonRoot":true,"runAsUser":10001,"runAsGroup":10001,"fsGroup":10001,"seccompProfile":{"type":"RuntimeDefault"}},"initContainers":[{"name":"initialize","image":s.image,"imagePullPolicy":"IfNotPresent","command":["/usr/local/bin/ayni-task-worker","initialize"],"env":env,"securityContext":security,"volumeMounts":[{"name":"work","mountPath":"/var/lib/ayni-task"},{"name":"tools","mountPath":"/opt/ayni-task"},{"name":"capability","mountPath":"/var/run/task-capability","readOnly":true},{"name":"temporary","mountPath":"/tmp"}],"resources":{"requests":{"cpu":"50m","memory":"64Mi"},"limits":{"cpu":"1","memory":"512Mi"}}}],"containers":[{"name":"coding","image":image,"imagePullPolicy":"IfNotPresent","command":["/opt/ayni-task/worker"],"env":env,"securityContext":security,"volumeMounts":mounts,"resources":{"requests":{"cpu":"100m","memory":"128Mi"},"limits":{"cpu":"1","memory":"1Gi"}}}],"volumes":[{"name":"work","persistentVolumeClaim":{"claimName":name}},{"name":"tools","emptyDir":{}},{"name":"temporary","emptyDir":{"sizeLimit":"256Mi"}},{"name":"capability","secret":{"secretName":name}}]}});
    // Initialization attaches the snapshot before the selected runtime sees the checkout.
    pod["spec"]["initContainers"][0]["env"][6]["value"] = json!("/var/lib/ayni-task/source");
    if command.provider == "openai" && s.openai {
        let secret = format!(
            "provider-openai-{}",
            uuid::Uuid::parse_str(&command.workspace_id)?.simple()
        );
        pod["spec"]["volumes"].as_array_mut().ok_or("volumes")?.push(json!({"name":"provider","secret":{"secretName":secret,"items":[{"key":"api-key","path":"api-key"}]}}));
        pod["spec"]["containers"][0]["volumeMounts"]
            .as_array_mut()
            .ok_or("mounts")?
            .push(json!({"name":"provider","mountPath":"/var/run/provider","readOnly":true}));
    }
    Ok(serde_json::from_value(pod)?)
}
