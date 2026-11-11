use super::{
    Api, DeleteParams, Failure, Pod, PostParams, Preconditions, PullCredential, StageClaim,
    Supervisor, delete_pod, pods,
};
use ayni_orchestrator_events::runtime_preparation::{Evidence, StageReport};
use k8s_openapi::{
    ByteString,
    api::core::v1::{PersistentVolumeClaim, Secret},
};
use kube::api::ListParams;
use serde_json::json;
use std::collections::BTreeMap;
async fn ensure_secret(
    s: &Supervisor,
    c: &StageClaim,
    name: String,
    kind: &str,
    data: BTreeMap<String, ByteString>,
) -> Result<(), Failure> {
    let api: Api<Secret> = Api::namespaced(s.client.clone(), &s.namespace);
    if let Some(existing) = api
        .get_opt(&name)
        .await
        .map_err(|error| s.kubernetes_failure("get_stage_secret", &error))?
    {
        if existing.immutable != Some(true)
            || existing.type_.as_deref() != Some(kind)
            || existing.data.as_ref() != Some(&data)
            || existing
                .metadata
                .labels
                .as_ref()
                .and_then(|l| l.get("ayni.build/operation"))
                != Some(&c.operation.id)
        {
            return Err(Failure::ArtifactConflict);
        }
        return Ok(());
    }
    let secret = Secret {
        metadata: kube::api::ObjectMeta {
            name: Some(name),
            labels: Some(
                serde_json::from_value(pods::labels(c)).map_err(|_| Failure::InvalidInput)?,
            ),
            ..Default::default()
        },
        immutable: Some(true),
        type_: Some(kind.into()),
        data: Some(data),
        ..Default::default()
    };
    api.create(&PostParams::default(), &secret)
        .await
        .map_err(|error| s.kubernetes_failure("create_stage_secret", &error))?;
    Ok(())
}
pub(super) async fn prepare(s: &Supervisor, c: &StageClaim) -> Result<(), Failure> {
    let stage = c.attempt.stage.as_str();
    let verify = matches!(stage, "verify_content" | "verify_compatibility");
    if stage == "certify" {
        verify_signer(s, c).await?;
    }
    if stage != "cleanup" && !verify {
        ensure_secret(
            s,
            c,
            pods::resource(c, &format!("cap-{stage}-{}", c.attempt.generation)),
            "Opaque",
            BTreeMap::from([("token".into(), ByteString(s.capability(c).into_bytes()))]),
        )
        .await?;
    }
    if verify {
        let credential: PullCredential = s
            .request(
                "/v1/runtime-supervisor/credential",
                &Supervisor::binding(c, ""),
            )
            .await?;
        if credential.kind != "registry" {
            return Err(Failure::InvalidInput);
        }
        let destination = &c.operation.frozen.registry_destination;
        let host = destination.split('/').next().ok_or(Failure::InvalidInput)?;
        let bytes=serde_json::to_vec(&json!({"auths":{host:{"username":credential.username,"password":credential.password}}})).map_err(|_|Failure::InvalidInput)?;
        ensure_secret(
            s,
            c,
            pods::resource(c, &format!("pull-{stage}-{}", c.attempt.generation)),
            "kubernetes.io/dockerconfigjson",
            BTreeMap::from([(".dockerconfigjson".into(), ByteString(bytes))]),
        )
        .await?;
    }
    if std::env::var_os("ORCHESTRATOR_TELEMETRY_ENDPOINT").is_some() && stage != "cleanup" {
        let api: Api<PersistentVolumeClaim> = Api::namespaced(s.client.clone(), &s.namespace);
        let name = pods::resource(c, &format!("telemetry-{stage}-{}", c.attempt.generation));
        if api
            .get_opt(&name)
            .await
            .map_err(|_| Failure::ExecutionFailed)?
            .is_none()
        {
            let mut value = json!({"apiVersion":"v1","kind":"PersistentVolumeClaim","metadata":{"name":name,"labels":pods::labels(c)},"spec":{"accessModes":["ReadWriteOnce"],"resources":{"requests":{"storage":"256Mi"}}}});
            if let Some(class) = &s.storage_class {
                value["spec"]["storageClassName"] = json!(class);
            }
            let pvc = serde_json::from_value(value).map_err(|_| Failure::InvalidInput)?;
            api.create(&PostParams::default(), &pvc)
                .await
                .map_err(|_| Failure::ExecutionFailed)?;
        }
    }
    let Some(role) = pods::output_role(c) else {
        return Ok(());
    };
    let api: Api<PersistentVolumeClaim> = Api::namespaced(s.client.clone(), &s.namespace);
    let name = pods::resource(c, role);
    if let Some(existing) = api
        .get_opt(&name)
        .await
        .map_err(|error| s.kubernetes_failure("get_stage_volume", &error))?
    {
        if existing
            .metadata
            .labels
            .as_ref()
            .and_then(|l| l.get("ayni.build/operation"))
            != Some(&c.operation.id)
        {
            return Err(Failure::ArtifactConflict);
        }
        return Ok(());
    }
    let profile: ayni_orchestrator_events::runtime_preparation::RuntimeProfile =
        serde_json::from_str(&c.operation.frozen.profile_json)
            .map_err(|_| Failure::InvalidInput)?;
    let mut object = json!({"apiVersion":"v1","kind":"PersistentVolumeClaim","metadata":{"name":name,"labels":pods::labels(c)},"spec":{"accessModes":["ReadWriteOnce"],"resources":{"requests":{"storage":format!("{}Mi",profile.max_storage_mib)}}}});
    if let Some(class) = &s.storage_class {
        object["spec"]["storageClassName"] = json!(class);
    }
    let pvc = serde_json::from_value(object).map_err(|_| Failure::InvalidInput)?;
    api.create(&PostParams::default(), &pvc)
        .await
        .map_err(|error| s.kubernetes_failure("create_stage_volume", &error))?;
    Ok(())
}
fn selector(c: &StageClaim) -> ListParams {
    ListParams::default().labels(&format!(
        "app.kubernetes.io/managed-by=ayni-runtime-supervisor,ayni.build/operation={}",
        c.operation.id
    ))
}
fn deletion(meta: &kube::api::ObjectMeta) -> DeleteParams {
    DeleteParams {
        preconditions: Some(Preconditions {
            uid: meta.uid.clone(),
            resource_version: meta.resource_version.clone(),
        }),
        ..Default::default()
    }
}
pub(super) async fn cleanup(s: &Supervisor, c: &StageClaim, sentinel: &Pod) -> Result<(), Failure> {
    let pods: Api<Pod> = Api::namespaced(s.client.clone(), &s.namespace);
    let secrets: Api<Secret> = Api::namespaced(s.client.clone(), &s.namespace);
    let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(s.client.clone(), &s.namespace);
    let mut pending = false;
    for pod in pods
        .list(&selector(c))
        .await
        .map_err(|_| Failure::CleanupPending)?
        .items
    {
        if pod
            .metadata
            .labels
            .as_ref()
            .is_some_and(|l| l.contains_key("ayni.build/telemetry-replay"))
        {
            continue;
        }
        if pod.metadata.uid != sentinel.metadata.uid {
            delete_pod(&pods, &pod).await?;
            pending = true;
        }
    }
    if pending {
        return Ok(());
    }
    for secret in secrets
        .list(&selector(c))
        .await
        .map_err(|_| Failure::CleanupPending)?
        .items
    {
        let name = secret
            .metadata
            .name
            .as_deref()
            .ok_or(Failure::InvalidInput)?;
        if name.contains("-telemetry-cap-") {
            continue;
        }
        secrets
            .delete(name, &deletion(&secret.metadata))
            .await
            .map_err(|_| Failure::CleanupPending)?;
        pending = true;
    }
    for pvc in pvcs
        .list(&selector(c))
        .await
        .map_err(|_| Failure::CleanupPending)?
        .items
    {
        let name = pvc.metadata.name.as_deref().ok_or(Failure::InvalidInput)?;
        if name.contains("-telemetry-") {
            continue;
        }
        pvcs.delete(name, &deletion(&pvc.metadata))
            .await
            .map_err(|_| Failure::CleanupPending)?;
        pending = true;
    }
    if pending {
        return Ok(());
    }
    let uid = sentinel
        .metadata
        .uid
        .as_deref()
        .ok_or(Failure::InvalidInput)?;
    s.worker(c, uid)?
        .report(StageReport {
            binding: Supervisor::binding(c, uid),
            outcome: "completed".into(),
            safe_reason: String::new(),
            evidence: Evidence {
                cleanup_complete: true,
                ..Default::default()
            },
        })
        .await
}

pub(super) async fn verify_signer(s: &Supervisor, c: &StageClaim) -> Result<(), Failure> {
    let api: Api<SignerMetadata> = Api::namespaced(s.client.clone(), &s.namespace);
    let secret = api
        .get(&c.operation.frozen.signer_secret_name)
        .await
        .map_err(|error| match error {
            kube::Error::Api(response) if response.code == 404 => Failure::CertificationFailed,
            _ => Failure::ExecutionFailed,
        })?;
    validate_signer(&secret, &c.operation.frozen.signer_secret_uid)
}
fn validate_signer(secret: &SignerMetadata, expected_uid: &str) -> Result<(), Failure> {
    if secret.metadata.uid.as_deref() != Some(expected_uid) || secret.immutable != Some(true) {
        return Err(Failure::CertificationFailed);
    }
    Ok(())
}

// Ignore credential data during deserialization; immutable is outside ObjectMeta,
// so Kubernetes PartialObjectMetadata cannot prove this condition.
#[derive(Clone, Debug, serde::Deserialize)]
struct SignerMetadata {
    metadata: kube::api::ObjectMeta,
    immutable: Option<bool>,
}
impl kube::Resource for SignerMetadata {
    type DynamicType = ();
    type Scope = k8s_openapi::NamespaceResourceScope;
    fn kind((): &()) -> std::borrow::Cow<'_, str> {
        "Secret".into()
    }
    fn group((): &()) -> std::borrow::Cow<'_, str> {
        "".into()
    }
    fn version((): &()) -> std::borrow::Cow<'_, str> {
        "v1".into()
    }
    fn plural((): &()) -> std::borrow::Cow<'_, str> {
        "secrets".into()
    }
    fn meta(&self) -> &kube::api::ObjectMeta {
        &self.metadata
    }
    fn meta_mut(&mut self) -> &mut kube::api::ObjectMeta {
        &mut self.metadata
    }
}

pub(super) async fn recover_capture(s: &Supervisor, c: &StageClaim) -> Result<(), Failure> {
    let Ok(endpoint) = std::env::var("ORCHESTRATOR_TELEMETRY_ENDPOINT") else {
        return Ok(());
    };
    let volume = pods::resource(
        c,
        &format!("telemetry-{}-{}", c.attempt.stage, c.attempt.generation),
    );
    let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(s.client.clone(), &s.namespace);
    if pvcs
        .get_opt(&volume)
        .await
        .map_err(|_| Failure::ExecutionFailed)?
        .is_none()
    {
        return Ok(());
    }
    let name = format!("{}-logs", pods::name(c)?);
    let api: Api<Pod> = Api::namespaced(s.client.clone(), &s.namespace);
    if api
        .get_opt(&name)
        .await
        .map_err(|_| Failure::ExecutionFailed)?
        .is_some()
    {
        return Ok(());
    }
    let secret = pods::resource(
        c,
        &format!("telemetry-cap-{}-{}", c.attempt.stage, c.attempt.generation),
    );
    ensure_secret(
        s,
        c,
        secret.clone(),
        "Opaque",
        BTreeMap::from([("token".into(), ByteString(s.capability(c).into_bytes()))]),
    )
    .await?;
    let pod = replay_spec(s, c, &name, &volume, &secret, &endpoint)?;
    api.create(&PostParams::default(), &pod)
        .await
        .map_err(|_| Failure::ExecutionFailed)?;
    Ok(())
}

fn replay_spec(
    s: &Supervisor,
    c: &StageClaim,
    name: &str,
    volume: &str,
    secret: &str,
    endpoint: &str,
) -> Result<Pod, Failure> {
    let profile: ayni_orchestrator_events::runtime_preparation::RuntimeProfile =
        serde_json::from_str(&c.operation.frozen.profile_json)
            .map_err(|_| Failure::InvalidInput)?;
    let replay_uid = if c.attempt.stage == "verify_compatibility" {
        0
    } else {
        1000
    };
    let mut labels = pods::labels(c);
    labels["ayni.build/telemetry-replay"] = json!("true");
    let pod=serde_json::from_value(json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":name,"labels":labels},"spec":{"hostUsers":false,"automountServiceAccountToken":false,"restartPolicy":"OnFailure","activeDeadlineSeconds":86400,"securityContext":{"runAsUser":replay_uid,"runAsGroup":replay_uid,"seccompProfile":{"type":"RuntimeDefault"}},"containers":[{"name":"replay","image":profile.worker_image,"imagePullPolicy":s.pull_policy,"command":["/usr/local/bin/ayni-runtime-worker","--replay-telemetry"],"env":[{"name":"ORCHESTRATOR_TELEMETRY_ENDPOINT","value":endpoint},{"name":"ORCHESTRATOR_TELEMETRY_SPOOL","value":"/telemetry"},{"name":"ORCHESTRATOR_RUNTIME_CAPABILITY_PATH","value":"/capability/token"}],"securityContext":{"allowPrivilegeEscalation":false,"readOnlyRootFilesystem":true,"capabilities":{"drop":["ALL"]}},"resources":{"requests":{"cpu":"10m","memory":"32Mi"},"limits":{"cpu":"250m","memory":"128Mi"}},"volumeMounts":[{"name":"spool","mountPath":"/telemetry"},{"name":"capability","mountPath":"/capability","readOnly":true}]}],"volumes":[{"name":"spool","persistentVolumeClaim":{"claimName":volume}},{"name":"capability","secret":{"secretName":secret}}]}})).map_err(|_|Failure::InvalidInput)?;
    Ok(pod)
}

pub(super) async fn arm_recovery(s: &Supervisor, c: &StageClaim) -> Result<(), Failure> {
    let Ok(endpoint) = std::env::var("ORCHESTRATOR_TELEMETRY_ENDPOINT") else {
        return Ok(());
    };
    if c.attempt.stage == "cleanup" {
        return Ok(());
    }
    let api: Api<PersistentVolumeClaim> = Api::namespaced(s.client.clone(), &s.namespace);
    let volume = pods::resource(
        c,
        &format!("telemetry-{}-{}", c.attempt.stage, c.attempt.generation),
    );
    let Some(pvc) = api
        .get_opt(&volume)
        .await
        .map_err(|_| Failure::ExecutionFailed)?
    else {
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
    let name = format!("{}-logs", pods::name(c)?);
    let secret = pods::resource(
        c,
        &format!("telemetry-cap-{}-{}", c.attempt.stage, c.attempt.generation),
    );
    ensure_secret(
        s,
        c,
        secret.clone(),
        "Opaque",
        BTreeMap::from([("token".into(), ByteString(s.capability(c).into_bytes()))]),
    )
    .await?;
    let spec = replay_spec(s, c, &name, &volume, &secret, &endpoint)?;
    let spec = serde_json::to_string(&spec).map_err(|_| Failure::InvalidInput)?;
    let patch = json!({"metadata":{"labels":{"ayni.build/telemetry-spool":"true"},"annotations":{"ayni.build/replay-spec":spec,"ayni.build/source-pod-uid":c.attempt.pod_uid}}});
    api.patch(
        &volume,
        &kube::api::PatchParams::default(),
        &kube::api::Patch::Merge(patch),
    )
    .await
    .map_err(|_| Failure::ExecutionFailed)?;
    Ok(())
}

async fn recover_orphans(
    s: &Supervisor,
    api: &Api<Pod>,
    pvcs: &Api<PersistentVolumeClaim>,
) -> Result<(), Failure> {
    for pvc in pvcs
        .list(&ListParams::default().labels(
            "app.kubernetes.io/managed-by=ayni-runtime-supervisor,ayni.build/telemetry-spool=true",
        ))
        .await
        .map_err(|_| Failure::ExecutionFailed)?
        .items
    {
        if pvc.metadata.deletion_timestamp.is_some() {
            continue;
        }
        let Some(annotations) = &pvc.metadata.annotations else {
            continue;
        };
        let Some(spec) = annotations.get("ayni.build/replay-spec") else {
            continue;
        };
        let mut replay: Pod = serde_json::from_str(spec).map_err(|_| Failure::InvalidInput)?;
        let name = replay.metadata.name.clone().ok_or(Failure::InvalidInput)?;
        let Some(original) = name.strip_suffix("-logs") else {
            continue;
        };
        if api
            .get_opt(&name)
            .await
            .map_err(|_| Failure::ExecutionFailed)?
            .is_some()
        {
            continue;
        }
        if let Some(pod) = api
            .get_opt(original)
            .await
            .map_err(|_| Failure::ExecutionFailed)?
        {
            if pod.metadata.uid.as_ref() != annotations.get("ayni.build/source-pod-uid")
                || !matches!(
                    pod.status.as_ref().and_then(|p| p.phase.as_deref()),
                    Some("Succeeded" | "Failed")
                )
            {
                continue;
            }
            if let Some(spec) = replay.spec.as_mut() {
                spec.node_name = pod.spec.as_ref().and_then(|p| p.node_name.clone());
            }
        }
        match api.create(&PostParams::default(), &replay).await {
            Ok(_) => {}
            Err(kube::Error::Api(e)) if e.code == 409 => {}
            Err(e) => return Err(s.kubernetes_failure("replay_capture", &e)),
        }
    }
    Ok(())
}

pub(super) async fn retire_replays(s: &Supervisor) -> Result<(), Failure> {
    let api: Api<Pod> = Api::namespaced(s.client.clone(), &s.namespace);
    let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(s.client.clone(), &s.namespace);
    let secrets: Api<Secret> = Api::namespaced(s.client.clone(), &s.namespace);
    recover_orphans(s, &api, &pvcs).await?;
    let list = api
        .list(&ListParams::default().labels(
            "app.kubernetes.io/managed-by=ayni-runtime-supervisor,ayni.build/telemetry-replay=true",
        ))
        .await
        .map_err(|_| Failure::ExecutionFailed)?;
    for pod in list.items {
        let phase = pod.status.as_ref().and_then(|s| s.phase.as_deref());
        if !matches!(phase, Some("Succeeded" | "Failed")) {
            continue;
        }
        if phase == Some("Failed") {
            eprintln!(
                "telemetry replay expired; capture incomplete for {}",
                pod.metadata.name.as_deref().unwrap_or_default()
            );
            // Preserve the failed recovery and its spool for explicit inspection/retry.
            continue;
        }
        if let Some(spec) = &pod.spec {
            for volume in spec.volumes.iter().flatten() {
                if let Some(pvc) = &volume.persistent_volume_claim
                    && pvc.claim_name.contains("-telemetry-")
                {
                    let _ = pvcs.delete(&pvc.claim_name, &DeleteParams::default()).await;
                }
                if let Some(secret) = &volume.secret
                    && let Some(name) = &secret.secret_name
                    && name.contains("-telemetry-cap-")
                {
                    let _ = secrets.delete(name, &DeleteParams::default()).await;
                }
            }
        }
        delete_pod(&api, &pod).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Failure, SignerMetadata, validate_signer};
    #[test]
    fn certification_requires_retained_immutable_secret_identity() {
        let mut view = SignerMetadata {
            metadata: kube::api::ObjectMeta {
                uid: Some("original-uid".into()),
                ..Default::default()
            },
            immutable: Some(true),
        };
        assert!(validate_signer(&view, "original-uid").is_ok());
        assert_eq!(
            validate_signer(&view, "replacement-uid"),
            Err(Failure::CertificationFailed)
        );
        view.immutable = Some(false);
        assert_eq!(
            validate_signer(&view, "original-uid"),
            Err(Failure::CertificationFailed)
        );
        view.immutable = None;
        assert_eq!(
            validate_signer(&view, "original-uid"),
            Err(Failure::CertificationFailed)
        );
    }
}
