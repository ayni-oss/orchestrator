//! Reporting tokens are mounted only into the trusted verifier, never checkout
//! or repository command authority. Existing Jobs keep their original contract.
use ayni_orchestrator_runtime::verification::REPORTING_AUDIENCE;
use k8s_openapi::api::batch::v1::Job;
use serde_json::json;

pub fn configure_job(job: &mut Job, event_authoritative: bool) -> Result<(), &'static str> {
    job.metadata.annotations.get_or_insert_default().insert(
        "ayni.io/verification-events".into(),
        if event_authoritative {
            "events-v1"
        } else {
            "shadow-v1"
        }
        .into(),
    );
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
    verifier
        .env
        .get_or_insert_default()
        .push(k8s_openapi::api::core::v1::EnvVar {
            name: "VERIFICATION_REPORTING_AUTHORITY".into(),
            value: Some(
                if event_authoritative {
                    "events"
                } else {
                    "shadow"
                }
                .into(),
            ),
            ..Default::default()
        });
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

/// Only trusted verifier inputs receive exporter configuration and launch context.
pub fn configure_telemetry(job: &mut Job) {
    let Some(endpoint) = ayni_orchestrator_observability::exporter_endpoint() else {
        return;
    };
    let context = ayni_orchestrator_observability::current_context();
    apply_telemetry(job, endpoint, context);
}

fn apply_telemetry(
    job: &mut Job,
    endpoint: String,
    context: Option<ayni_orchestrator_observability::TraceContext>,
) {
    let Some(verifier) = job
        .spec
        .as_mut()
        .and_then(|s| s.template.spec.as_mut())
        .and_then(|pod| pod.containers.iter_mut().find(|c| c.name == "verify"))
    else {
        return;
    };
    let env = verifier.env.get_or_insert_default();
    env.push(k8s_openapi::api::core::v1::EnvVar {
        name: "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT".into(),
        value: Some(endpoint),
        ..Default::default()
    });
    if let Some(context) = context {
        env.push(k8s_openapi::api::core::v1::EnvVar {
            name: "TRACEPARENT".into(),
            value: Some(context.traceparent),
            ..Default::default()
        });
        if let Some(state) = context.tracestate {
            env.push(k8s_openapi::api::core::v1::EnvVar {
                name: "TRACESTATE".into(),
                value: Some(state),
                ..Default::default()
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn telemetry_inputs_are_isolated_from_checkout_and_repository_children()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut job: Job = serde_json::from_value(
            json!({"metadata":{},"spec":{"template":{"spec":{"initContainers":[{"name":"checkout","image":"trusted"}],"containers":[{"name":"verify","image":"pinned"},{"name":"repository","image":"pinned"}]}}}}),
        )?;
        let context = ayni_orchestrator_observability::TraceContext::parse(
            "00-7431fc7927bb4f208986f2a4f2418dba-3f49c016c2df402b-01",
            None,
        )
        .ok_or("context")?;
        apply_telemetry(
            &mut job,
            "http://collector:4318/v1/traces".into(),
            Some(context.clone()),
        );
        let pod = job
            .spec
            .as_ref()
            .and_then(|s| s.template.spec.as_ref())
            .ok_or("pod")?;
        assert!(
            pod.init_containers.as_ref().ok_or("checkout")?[0]
                .env
                .is_none()
        );
        assert!(pod.containers[1].env.is_none());
        let env = pod.containers[0].env.as_ref().ok_or("env")?;
        assert!(
            env.iter().any(|e| e.name == "TRACEPARENT"
                && e.value.as_deref() == Some(context.traceparent.as_str()))
        );
        assert!(
            env.iter()
                .any(|e| e.name == "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT")
        );
        Ok(())
    }
    #[test]
    fn reporting_identity_is_projected_only_to_the_supervisor()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut job: Job = serde_json::from_value(
            json!({"metadata":{},"spec":{"template":{"spec":{"automountServiceAccountToken":false,"initContainers":[{"name":"checkout","image":"trusted"}],"containers":[{"name":"verify","image":"pinned"}],"volumes":[]}}}}),
        )?;
        configure_job(&mut job, false)?;
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
    #[test]
    fn selected_reporting_authority_is_confined_to_the_verifier()
    -> Result<(), Box<dyn std::error::Error>> {
        for event_authoritative in [false, true] {
            let mut job: Job = serde_json::from_value(json!({
                "metadata": {}, "spec": {"template": {"spec": {
                    "initContainers": [{"name": "checkout", "image": "trusted"}],
                    "containers": [{"name": "verify", "image": "pinned"},
                                   {"name": "repository", "image": "pinned"}]
                }}}
            }))?;
            configure_job(&mut job, event_authoritative)?;
            let pod = job
                .spec
                .as_ref()
                .and_then(|s| s.template.spec.as_ref())
                .ok_or("pod")?;
            let expected = if event_authoritative {
                "events"
            } else {
                "shadow"
            };
            let verifier = pod
                .containers
                .iter()
                .find(|c| c.name == "verify")
                .ok_or("verify")?;
            assert!(
                verifier
                    .env
                    .as_ref()
                    .ok_or("env")?
                    .iter()
                    .any(|e| e.name == "VERIFICATION_REPORTING_AUTHORITY"
                        && e.value.as_deref() == Some(expected))
            );
            assert!(
                pod.containers
                    .iter()
                    .filter(|c| c.name != "verify")
                    .all(|c| c.env.is_none())
            );
            assert!(
                pod.init_containers
                    .as_ref()
                    .ok_or("checkout")?
                    .iter()
                    .all(|c| c.env.is_none())
            );
        }
        Ok(())
    }
}
