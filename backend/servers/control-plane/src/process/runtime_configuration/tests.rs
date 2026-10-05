use super::*;
fn configuration() -> RuntimeConfiguration {
    RuntimeConfiguration {
        namespace: "test".into(),
        output_claim: "output".into(),
        model_secret: "model".into(),
        worker_image: format!("registry.test/worker@sha256:{}", "a".repeat(64)),
        interval_seconds: 60,
        request_timeout_seconds: 30,
        concurrency: 4,
        allow_emulation: false,
        profiles: BTreeMap::new(),
        managed_runtime: None,
    }
}

#[test]
fn rejects_unbounded_scheduling_and_credential_values() {
    let mut configuration = configuration();
    assert!(configuration.validate().is_ok());
    configuration.concurrency = 0;
    assert!(configuration.validate().is_err());
    configuration.concurrency = 4;
    for secret in ["token:value", "../secret"] {
        configuration.model_secret = secret.into();
        assert!(configuration.validate().is_err());
    }
}

#[test]
fn managed_discovery_uses_the_operator_registry_endpoint() -> Result<(), &'static str> {
    let mut configuration = configuration();
    let owner = "Example";
    let repository = "Sandbox";
    let oci_repository = "ghcr.io/example/sandbox-ayni";
    let lock_fingerprint = "a".repeat(64);
    assert!(
        configuration
            .managed_runtime(owner, repository, &lock_fingerprint, oci_repository)
            .is_none()
    );
    configuration.managed_runtime = Some(ManagedRuntime {
        registry_prefix: "registry.test/runtimes".into(),
        platform: "linux/amd64".into(),
        service_account: "runtime-coding".into(),
        pull_secret: Some("runtime-pull".into()),
        build: None,
        assessment: None,
    });
    assert!(configuration.validate().is_ok());
    let runtime = configuration
        .managed_runtime(owner, repository, &lock_fingerprint, oci_repository)
        .ok_or("managed runtime missing")?;
    assert_eq!(runtime.pull_secret.as_deref(), Some("runtime-pull"));
    assert_eq!(
        runtime.image,
        format!(
            "registry.test/runtimes/example/sandbox-ayni:lock-{}",
            "a".repeat(64)
        )
    );
    assert!(
        configuration
            .managed_runtime(owner, repository, "invalid", oci_repository)
            .is_none()
    );
    Ok(())
}

#[test]
fn derives_managed_image_from_connected_repository_and_lock() -> Result<(), &'static str> {
    let runtime = ManagedRuntime {
        registry_prefix: "ghcr.io".into(),
        platform: "linux/amd64".into(),
        service_account: "runtime-coding".into(),
        pull_secret: Some("environment-pull".into()),
        build: None,
        assessment: None,
    };
    let profile = runtime.profile_runtime("Ayni-Oss", "sandbox", &"aB".repeat(32))?;
    assert_eq!(
        profile.image,
        format!("ghcr.io/ayni-oss/sandbox-ayni:lock-{}", "ab".repeat(32))
    );
    assert!(
        runtime
            .profile_runtime("../owner", "sandbox", &"a".repeat(64))
            .is_err()
    );
    Ok(())
}

fn operator_document() -> serde_json::Value {
    serde_json::json!({
        "namespace": "runtime-test",
        "worker_image": format!("registry.test/worker@sha256:{}", "a".repeat(64)),
        "output_claim": "outputs", "model_secret": "model", "allow_emulation": true,
        "interval_seconds": 42, "request_timeout_seconds": 12, "concurrency": 3,
        "profiles": {
            "project-profile": {
                "image": "registry.test/environment:lock", "platform": "linux/arm64",
                "service_account": "coding", "pull_secret": "pull",
                "build": {
                    "image": format!("registry.test/builder@sha256:{}", "b".repeat(64)),
                    "service_account": "builder", "registry_secret": "push",
                    "signing_secret": "signing", "engine_secret": "engine",
                    "engine_host": "tcp://engine.test:2376"
                },
                "assessment": {
                    "producer_image": format!("registry.test/collector@sha256:{}", "c".repeat(64)),
                    "publication_policy": "READY_AFTER_SUCCESSFUL_ASSESSMENT"
                }
            }
        },
        "managed_runtime": {
            "registry_prefix": "ghcr.io", "platform": "linux/amd64", "service_account": "managed", "pull_secret": "pull", "build": null,
            "assessment": {
                "producer_image": format!("registry.test/collector@sha256:{}", "c".repeat(64)),
                "publication_policy": "DRAFT_ONLY"
            }
        }
    })
}

#[test]
fn operator_json_and_execution_projection_preserve_configured_values()
-> Result<(), Box<dyn std::error::Error>> {
    let document = operator_document();
    let configuration: RuntimeConfiguration = serde_json::from_value(document.clone())?;
    configuration.validate()?;
    assert_eq!(serde_json::to_value(&configuration)?, document);
    let execution = configuration.execution_configuration();
    assert_eq!(execution.namespace, configuration.namespace);
    assert_eq!(execution.worker_image, configuration.worker_image);
    assert_eq!(execution.output_claim, configuration.output_claim);
    assert_eq!(execution.model_secret, configuration.model_secret);
    assert!(execution.allow_emulation);
    let profile = configuration
        .profiles
        .get("project-profile")
        .ok_or("profile")?;
    let workload = profile.execution_runtime();
    assert_eq!(workload.image, profile.image);
    assert_eq!(workload.platform, profile.platform);
    assert_eq!(workload.service_account, profile.service_account);
    assert_eq!(workload.pull_secret, profile.pull_secret);
    assert_eq!(
        serde_json::to_value(workload.build)?,
        serde_json::to_value(&profile.build)?
    );
    Ok(())
}

#[test]
fn operator_json_rejects_unknown_fields_at_every_configuration_boundary()
-> Result<(), Box<dyn std::error::Error>> {
    for pointer in [
        "",
        "/profiles/project-profile",
        "/profiles/project-profile/build",
        "/profiles/project-profile/assessment",
        "/managed_runtime",
        "/managed_runtime/assessment",
    ] {
        let mut document = operator_document();
        document
            .pointer_mut(pointer)
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("configuration object")?
            .insert("unrecognized_setting".into(), serde_json::json!(true));
        assert!(
            serde_json::from_value::<RuntimeConfiguration>(document).is_err(),
            "accepted unknown field at {pointer}"
        );
    }
    Ok(())
}

#[test]
fn operator_json_preserves_scheduling_and_workload_defaults()
-> Result<(), Box<dyn std::error::Error>> {
    let configuration: RuntimeConfiguration = serde_json::from_value(serde_json::json!({
        "namespace": "runtime-test", "output_claim": "outputs", "model_secret": "model",
        "worker_image": format!("registry.test/worker@sha256:{}", "a".repeat(64)),
        "profiles": {"default": {
            "image": "registry.test/environment:lock", "platform": "linux/amd64",
            "build": {
                "image": format!("registry.test/builder@sha256:{}", "b".repeat(64)),
                "registry_secret": "push", "signing_secret": "signing", "engine_secret": "engine",
                "engine_host": "tcp://engine.test:2376"
            },
            "assessment": {"producer_image": format!("registry.test/collector@sha256:{}", "c".repeat(64))}
        }}
    }))?;
    configuration.validate()?;
    assert_eq!(configuration.interval_seconds, 60);
    assert_eq!(configuration.request_timeout_seconds, 30);
    assert_eq!(configuration.concurrency, 4);
    assert!(!configuration.allow_emulation);
    assert!(configuration.managed_runtime.is_none());
    let profile = configuration.profiles.get("default").ok_or("profile")?;
    assert_eq!(profile.service_account, "runtime-coding");
    assert!(profile.pull_secret.is_none());
    assert_eq!(
        profile.build.as_ref().ok_or("build")?.service_account,
        "runtime-builder"
    );
    assert!(
        profile
            .assessment
            .as_ref()
            .ok_or("assessment")?
            .publication_policy
            == PublicationPolicy::DraftOnly
    );
    Ok(())
}

#[test]
fn operator_validation_preserves_security_failure_codes() -> Result<(), Box<dyn std::error::Error>>
{
    for (pointer, value, reason) in [
        (
            "/worker_image",
            "registry.test/worker:latest",
            "runtime_configuration_invalid",
        ),
        (
            "/profiles/project-profile/platform",
            "darwin/arm64",
            "runtime_configuration_invalid",
        ),
        (
            "/profiles/project-profile/pull_secret",
            "../secret",
            "runtime_configuration_invalid",
        ),
        (
            "/profiles/project-profile/build/image",
            "registry.test/builder:latest",
            "runtime_build_configuration_invalid",
        ),
        (
            "/profiles/project-profile/build/engine_host",
            "tcp://engine.test:2375",
            "runtime_build_configuration_invalid",
        ),
        (
            "/profiles/project-profile/build/signing_secret",
            "private:key",
            "runtime_build_configuration_invalid",
        ),
        (
            "/profiles/project-profile/assessment/producer_image",
            "registry.test/collector:latest",
            "assessment_producer_invalid",
        ),
    ] {
        let mut document = operator_document();
        *document.pointer_mut(pointer).ok_or("configuration field")? = serde_json::json!(value);
        let configuration: RuntimeConfiguration = serde_json::from_value(document)?;
        assert_eq!(
            configuration.validate(),
            Err(reason),
            "unexpected error for {pointer}"
        );
    }
    Ok(())
}
