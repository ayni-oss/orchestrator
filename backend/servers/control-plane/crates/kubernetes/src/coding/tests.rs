use super::*;

fn fixture() -> (ExecutionConfiguration, WorkloadRuntime) {
    (
        ExecutionConfiguration {
            namespace: "runtime-test".into(),
            worker_image: format!("registry.test/worker@sha256:{}", "a".repeat(64)),
            output_claim: "outputs".into(),
            model_secret: "model".into(),
            allow_emulation: false,
        },
        WorkloadRuntime {
            image: "registry.test/team/runtime:lock".into(),
            platform: "linux/amd64".into(),
            service_account: "runtime-coding".into(),
            pull_secret: Some("pull-only".into()),
            build: Some(crate::configuration::BuildRuntime {
                image: format!("registry.test/builder@sha256:{}", "b".repeat(64)),
                service_account: "runtime-builder".into(),
                engine_host: "tcp://engine.test:2376".into(),
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
