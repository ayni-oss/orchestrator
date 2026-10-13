use super::*;

fn fixture() -> (ExecutionConfiguration, WorkloadRuntime) {
    (
        ExecutionConfiguration {
            namespace: "runtime-test".into(),
            worker_image: format!("registry.test/worker@sha256:{}", "a".repeat(64)),
            output_claim: "outputs".into(),
            model_secret: "model".into(),
            allow_emulation: false,
            engine_mode: "external-isolated".into(),
        },
        WorkloadRuntime {
            operation_reporting: None,
            image: "registry.test/team/runtime:lock".into(),
            platform: "linux/amd64".into(),
            service_account: "runtime-coding".into(),
            pull_secret: Some("pull-only".into()),
            build: Some(crate::configuration::BuildRuntime {
                configuration_revision: 0,
                image: format!("ghcr.io/ayni-oss/ayni-builder@sha256:{}", "b".repeat(64)),
                service_account: "runtime-builder".into(),
                engine_host: "tcp://engine.test:2376".into(),
                isolated_engine: true,
                engine: None,
                engine_secret: "engine-only".into(),
                registry_secret: "push-only".into(),
                signing_secret: "signing-only".into(),
            }),
        },
    )
}

#[test]
fn coding_uses_shared_digest_launch_without_build_authority()
-> Result<(), Box<dyn std::error::Error>> {
    let (configuration, runtime) = fixture();
    let image = format!("registry.test/team/runtime@sha256:{}", "c".repeat(64));
    let job = coding_job("coding-test", &configuration, &runtime, &image, 3600)?;
    let pod = job.spec.ok_or("spec")?.template.spec.ok_or("pod")?;
    assert!(
        pod.containers[0]
            .volume_mounts
            .iter()
            .flatten()
            .any(|m| m.name == "request"
                && m.mount_path == "/request/worker.json"
                && m.sub_path.as_deref() == Some("worker.json")
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
    let (configuration, runtime) = fixture();
    let job = build_job("build-test", &configuration, &runtime)?;
    let spec = job.spec.ok_or("spec")?;
    assert_eq!(spec.backoff_limit, Some(0));
    assert_eq!(spec.active_deadline_seconds, Some(1800));
    let pod = spec.template.spec.ok_or("pod")?;
    assert_eq!(pod.service_account_name.as_deref(), Some("runtime-builder"));
    assert_eq!(pod.automount_service_account_token, Some(false));
    let producer = &pod.containers[0];
    assert_eq!(
        producer.command.as_deref(),
        Some(["/launcher/runtime-worker".to_owned()].as_slice())
    );
    assert_eq!(
        producer
            .env
            .as_ref()
            .and_then(|values| values
                .iter()
                .find(|value| value.name == "AYNI_BUILDER_IMAGE"))
            .and_then(|value| value.value.as_ref()),
        producer.image.as_ref()
    );
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
        "pull-only",
        "signing-only",
        "engine-only",
    ] {
        assert!(json.contains(required));
    }
    assert!(!json.contains("model"));
    assert!(!json.contains("push-only"));
    Ok(())
}

#[test]
fn worker_credentials_are_refreshable_and_build_credentials_remain_in_checkout()
-> Result<(), Box<dyn std::error::Error>> {
    let (configuration, runtime) = fixture();
    let image = format!("registry.test/team/runtime@sha256:{}", "c".repeat(64));
    for (name, job) in [
        (
            "coding-test",
            coding_job("coding-test", &configuration, &runtime, &image, 3600)?,
        ),
        (
            "build-test",
            build_job("build-test", &configuration, &runtime)?,
        ),
        (
            "verify-test",
            crate::execution::verification_job("verify-test", &configuration, &runtime)?,
        ),
    ] {
        let pod = job.spec.ok_or("spec")?.template.spec.ok_or("pod")?;
        let source_users: Vec<_> = pod
            .containers
            .iter()
            .chain(pod.init_containers.iter().flatten())
            .filter(|container| {
                container.env.iter().flatten().any(|variable| {
                    variable
                        .value_from
                        .as_ref()
                        .and_then(|source| source.secret_key_ref.as_ref())
                        .is_some_and(|secret| secret.name == name && secret.key == "token")
                })
            })
            .map(|container| container.name.as_str())
            .collect();
        if name == "coding-test" {
            assert!(source_users.is_empty());
            assert!(
                pod.init_containers
                    .iter()
                    .flatten()
                    .all(|c| c.name != "checkout")
            );
            let credential = pod.containers[0]
                .volume_mounts
                .iter()
                .flatten()
                .find(|m| m.name == "credentials")
                .ok_or("credentials")?;
            assert_eq!(credential.mount_path, "/source");
            assert!(credential.sub_path.is_none());
            assert_eq!(credential.read_only, Some(true));
            assert!(pod.volumes.iter().flatten().any(|v| {
                v.secret
                    .as_ref()
                    .is_some_and(|s| s.secret_name.as_deref() == Some(name))
            }));
        } else {
            assert_eq!(source_users, ["checkout"]);
            assert!(pod.volumes.iter().flatten().all(|volume| {
                volume
                    .secret
                    .as_ref()
                    .is_none_or(|secret| secret.secret_name.as_deref() != Some(name))
            }));
        }
    }
    Ok(())
}

#[test]
fn coding_output_root_is_confined_to_output_preparation() -> Result<(), Box<dyn std::error::Error>>
{
    let (configuration, runtime) = fixture();
    let image = format!("registry.test/team/runtime@sha256:{}", "c".repeat(64));
    let job = coding_job("coding-test", &configuration, &runtime, &image, 3600)?;
    let pod = job.spec.ok_or("spec")?.template.spec.ok_or("pod")?;
    let agent = &pod.containers[0];
    let output = agent
        .volume_mounts
        .iter()
        .flatten()
        .find(|mount| mount.name == "output")
        .ok_or("agent output")?;
    assert_eq!(output.mount_path, "/output");
    assert_eq!(output.sub_path.as_deref(), Some("coding-test"));
    let root_mounts: Vec<_> = pod
        .init_containers
        .iter()
        .flatten()
        .flat_map(|container| {
            container
                .volume_mounts
                .iter()
                .flatten()
                .filter(|mount| mount.name == "output")
                .map(|mount| (container.name.as_str(), mount))
        })
        .collect();
    assert_eq!(root_mounts.len(), 1);
    assert_eq!(root_mounts[0].0, "output");
    assert_eq!(root_mounts[0].1.mount_path, "/outputs");
    assert!(root_mounts[0].1.sub_path.is_none());
    Ok(())
}

#[test]
fn publication_runs_without_source_or_signing_authority() -> Result<(), Box<dyn std::error::Error>>
{
    let (configuration, runtime) = fixture();
    let candidate = format!("sha256:{}", "d".repeat(64));
    let job = publication_job("publish-test", &configuration, &runtime, &candidate, false)?;
    let spec = job.spec.ok_or("spec")?;
    assert_eq!(spec.backoff_limit, Some(0));
    let pod = spec.template.spec.ok_or("pod")?;
    assert_eq!(pod.automount_service_account_token, Some(false));
    let json = serde_json::to_string(&pod)?;
    for forbidden in [
        "SOURCE_TOKEN",
        "checkout",
        "signing-only",
        "AYNI_ENV_CERTIFICATE",
        "hostPath",
        "persistentVolumeClaim",
        "privileged",
    ] {
        assert!(
            !json.contains(forbidden),
            "unexpected authority: {forbidden}"
        );
    }
    for required in ["push-only", "engine-only", "BUILD_IMAGE_ID", &candidate] {
        assert!(json.contains(required));
    }
    assert!(
        publication_job(
            "publish-test",
            &configuration,
            &runtime,
            "mutable:tag",
            false
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn attempt_jobs_project_only_client_tls_and_freeze_the_infrastructure_revision()
-> Result<(), Box<dyn std::error::Error>> {
    let (mut configuration, mut runtime) = fixture();
    configuration.engine_mode = "local-development".into();
    let build = runtime.build.as_mut().ok_or("build")?;
    build.configuration_revision = 7;
    build.image = format!("ghcr.io/ayni-oss/ayni-builder@sha256:{}", "b".repeat(64));
    build.engine = Some(
        ayni_orchestrator_application::execution_settings::EngineRuntime {
            image: format!("docker.io/library/docker@sha256:{}", "c".repeat(64)),
            security_mode: "local-development".into(),
        },
    );
    let runtime = crate::engine::bind(&runtime, "attempt-one");
    for job in [
        build_job("attempt-one", &configuration, &runtime)?,
        publication_job(
            "publisher",
            &configuration,
            &runtime,
            &format!("sha256:{}", "d".repeat(64)),
            false,
        )?,
    ] {
        let template = job.spec.ok_or("job")?.template;
        let metadata = template.metadata.ok_or("metadata")?;
        assert_eq!(
            metadata
                .labels
                .ok_or("labels")?
                .get("ayni.io/engine-session")
                .map(String::as_str),
            Some("attempt-one")
        );
        assert_eq!(
            metadata
                .annotations
                .ok_or("annotations")?
                .get("ayni.io/build-settings-revision")
                .map(String::as_str),
            Some("7")
        );
        let pod = template.spec.ok_or("pod")?;
        let affinity = serde_json::to_value(pod.affinity.as_ref().ok_or("trusted placement")?)?;
        let expression = &affinity["nodeAffinity"]["requiredDuringSchedulingIgnoredDuringExecution"]
            ["nodeSelectorTerms"][0]["matchExpressions"][0];
        assert_eq!(expression["key"], crate::workload::BUILDER_NODE_LABEL);
        assert_eq!(expression["operator"], "DoesNotExist");
        assert_eq!(
            pod.containers[0].image_pull_policy.as_deref(),
            Some("Always")
        );
        let security = pod.containers[0]
            .security_context
            .as_ref()
            .ok_or("security")?;
        assert_eq!(security.run_as_user, Some(10001));
        assert_eq!(security.read_only_root_filesystem, Some(true));
        let tls = pod
            .volumes
            .as_ref()
            .ok_or("volumes")?
            .iter()
            .find(|v| v.name == "engine")
            .and_then(|v| v.secret.as_ref())
            .ok_or("tls")?;
        assert_eq!(tls.secret_name.as_deref(), Some("attempt-one-engine"));
        let keys: Vec<_> = tls
            .items
            .as_ref()
            .ok_or("items")?
            .iter()
            .map(|i| i.key.as_str())
            .collect();
        assert_eq!(keys, ["ca.pem", "cert.pem", "key.pem"]);
        assert!(!serde_json::to_string(&pod)?.contains("server-key.pem"));
    }
    Ok(())
}

#[test]
fn certified_runtime_requires_signing_authority() -> Result<(), Box<dyn std::error::Error>> {
    let (configuration, mut runtime) = fixture();
    runtime
        .build
        .as_mut()
        .ok_or("build")?
        .signing_secret
        .clear();
    assert!(build_job("runtime", &configuration, &runtime).is_err());
    Ok(())
}

#[test]
fn coding_mounts_only_the_selected_model_credential_revision()
-> Result<(), Box<dyn std::error::Error>> {
    let (mut configuration, runtime) = fixture();
    configuration.model_secret = "openai-frozen-revision".into();
    let image = format!("registry.test/team/runtime@sha256:{}", "c".repeat(64));
    let job = coding_job("coding-selected", &configuration, &runtime, &image, 3600)?;
    let pod = job.spec.ok_or("spec")?.template.spec.ok_or("pod")?;
    let model_secrets = pod
        .volumes
        .iter()
        .flatten()
        .filter(|volume| volume.name == "model")
        .filter_map(|volume| {
            volume
                .secret
                .as_ref()
                .and_then(|secret| secret.secret_name.as_deref())
        })
        .collect::<Vec<_>>();
    assert_eq!(model_secrets, ["openai-frozen-revision"]);
    assert!(
        !pod.init_containers
            .iter()
            .flatten()
            .flat_map(|container| container.volume_mounts.iter().flatten())
            .any(|mount| mount.name == "model")
    );
    Ok(())
}
