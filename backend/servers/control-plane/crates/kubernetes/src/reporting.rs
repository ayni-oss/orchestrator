//! Reporting tokens are mounted only into the trusted verifier, never checkout
//! or repository command authority. Existing Jobs keep their original contract.
use ayni_orchestrator_runtime::verification::REPORTING_AUDIENCE;
use k8s_openapi::api::batch::v1::Job;
use serde_json::json;

pub fn configure_job(job: &mut Job) -> Result<(), &'static str> {
    job.metadata
        .annotations
        .get_or_insert_default()
        .insert("ayni.io/verification-events".into(), "shadow-v1".into());
    let pod = job
        .spec
        .as_mut()
        .and_then(|spec| spec.template.spec.as_mut())
        .ok_or("runtime_job_invalid")?;
    let verifier = pod
        .containers
        .iter_mut()
        .find(|container| container.name == "verify")
        .ok_or("runtime_job_invalid")?;
    let mounts = verifier.volume_mounts.get_or_insert_default();
    mounts.push(serde_json::from_value(json!({"name":"request","mountPath":"/request/reporting.json","subPath":"reporting.json","readOnly":true})).map_err(|_| "runtime_job_invalid")?);
    mounts.push(
        serde_json::from_value(
            json!({"name":"reporting-identity","mountPath":"/reporting","readOnly":true}),
        )
        .map_err(|_| "runtime_job_invalid")?,
    );
    pod.volumes.get_or_insert_default().push(serde_json::from_value(json!({"name":"reporting-identity","projected":{"defaultMode":288,"sources":[{"serviceAccountToken":{"audience":REPORTING_AUDIENCE,"expirationSeconds":600,"path":"token"}}]}})).map_err(|_| "runtime_job_invalid")?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reporting_identity_is_projected_only_to_the_supervisor()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut job: Job = serde_json::from_value(
            json!({"metadata":{},"spec":{"template":{"spec":{"automountServiceAccountToken":false,"initContainers":[{"name":"checkout","image":"trusted"}],"containers":[{"name":"verify","image":"pinned"}],"volumes":[]}}}}),
        )?;
        configure_job(&mut job)?;
        let pod = job
            .spec
            .as_ref()
            .and_then(|spec| spec.template.spec.as_ref())
            .ok_or("pod")?;
        assert_eq!(pod.automount_service_account_token, Some(false));
        assert!(
            pod.init_containers
                .as_ref()
                .ok_or("init")?
                .iter()
                .all(
                    |container| container.volume_mounts.as_ref().is_none_or(|mounts| mounts
                        .iter()
                        .all(|mount| mount.name != "reporting-identity"))
                )
        );
        let mounts = pod.containers[0].volume_mounts.as_ref().ok_or("mounts")?;
        assert!(
            mounts
                .iter()
                .any(|mount| mount.mount_path == "/reporting" && mount.read_only == Some(true))
        );
        let volume = pod
            .volumes
            .as_ref()
            .ok_or("volumes")?
            .iter()
            .find(|volume| volume.name == "reporting-identity")
            .ok_or("token")?;
        let projection = volume
            .projected
            .as_ref()
            .and_then(|projection| projection.sources.as_ref())
            .and_then(|sources| sources.first())
            .and_then(|source| source.service_account_token.as_ref())
            .ok_or("projection")?;
        assert_eq!(projection.audience.as_deref(), Some(REPORTING_AUDIENCE));
        assert_eq!(projection.expiration_seconds, Some(600));
        Ok(())
    }
}
