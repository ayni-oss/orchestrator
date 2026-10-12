use super::*;
fn configuration() -> RuntimeConfiguration {
    RuntimeConfiguration {
        reporting: None,
        namespace: "test".into(),
        output_claim: "output".into(),
        model_secret: "model".into(),
        worker_image: format!("registry.test/worker@sha256:{}", "a".repeat(64)),
        interval_seconds: 60,
        request_timeout_seconds: 30,
        coding_turns: 24,
        quality_repair_limit: 3,
        concurrency: 4,
        allow_emulation: false,
        engine_mode: "external-isolated".into(),

        profiles: BTreeMap::new(),
        managed_runtime: None,
    }
}

#[test]
fn rejects_unbounded_scheduling_and_credential_values() {
    let mut configuration = configuration();
    assert!(configuration.validate().is_ok());
    configuration.coding_turns = 0;
    assert!(configuration.validate().is_err());
    configuration.coding_turns = 129;
    assert!(configuration.validate().is_err());
    configuration.coding_turns = 24;
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
    let oci_repository = "ghcr.io/example/sandbox";
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
    });
    assert!(configuration.validate().is_ok());
    let runtime = configuration
        .managed_runtime(owner, repository, &lock_fingerprint, oci_repository)
        .ok_or("managed runtime missing")?;
    assert_eq!(runtime.pull_secret.as_deref(), Some("runtime-pull"));
    assert_eq!(
        runtime.image,
        format!(
            "registry.test/runtimes/example/sandbox:ayni-amd64-{}",
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
    };
    let profile = runtime.profile_runtime("Ayni-Oss", "sandbox", &"aB".repeat(32))?;
    assert_eq!(
        profile.image,
        format!("ghcr.io/ayni-oss/sandbox:ayni-amd64-{}", "ab".repeat(32))
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
        "output_claim": "outputs", "model_secret": "model", "allow_emulation": true, "engine_mode": "external-isolated",
        "interval_seconds": 42, "request_timeout_seconds": 12, "coding_turns": 17, "quality_repair_limit": 3, "concurrency": 3,
        "profiles": {
            "project-profile": {
                "image": "registry.test/environment:lock", "platform": "linux/arm64",
                "service_account": "coding", "pull_secret": "pull",
                "build": {
                    "configuration_revision": 0,
                    "image": "",
                    "service_account": "builder", "registry_secret": "push",
                    "signing_secret": "signing", "engine_secret": "engine",
                    "engine_host": "tcp://engine.test:2376", "isolated_engine": true
                }
            }
        },
        "managed_runtime": {
            "registry_prefix": "ghcr.io", "platform": "linux/amd64", "service_account": "managed", "pull_secret": "pull", "build": null
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
        "/managed_runtime",
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
                "image": "",
                "registry_secret": "push", "signing_secret": "signing", "engine_secret": "engine",
                "engine_host": "tcp://engine.test:2376", "isolated_engine": true
            }
        }}
    }))?;
    configuration.validate()?;
    assert_eq!(configuration.interval_seconds, 60);
    assert_eq!(configuration.request_timeout_seconds, 30);
    assert_eq!(configuration.coding_turns, 24);
    assert_eq!(configuration.quality_repair_limit, 3);
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

#[test]
fn quality_repairs_are_bounded() {
    let mut config = configuration();
    for limit in [0, 3, 10] {
        config.quality_repair_limit = limit;
        assert!(config.validate().is_ok());
    }
    config.quality_repair_limit = 11;
    assert!(config.validate().is_err());
}

#[test]
fn connection_destinations_remove_only_a_repeated_source_owner() -> Result<(), &'static str> {
    assert_eq!(
        connection_image_repository("ayni-oss", "AYNI-OSS", "Sandbox")?,
        "ghcr.io/ayni-oss/sandbox"
    );
    assert_eq!(
        connection_image_repository("runtimes", "ayni-oss", "sandbox")?,
        "ghcr.io/runtimes/ayni-oss/sandbox"
    );
    assert_eq!(
        connection_image_repository("ayni-oss/runtime", "ayni-oss", "sandbox")?,
        "ghcr.io/ayni-oss/runtime/ayni-oss/sandbox"
    );
    assert_ne!(
        connection_image_repository("runtimes", "a-b", "c")?,
        connection_image_repository("runtimes", "a", "b-c")?
    );
    assert!(connection_image_repository("https://untrusted", "owner", "repo").is_err());
    Ok(())
}

#[test]
fn architecture_tags_preserve_the_entire_lock_and_enforce_bounds() -> Result<(), &'static str> {
    let hash = "a".repeat(64);
    for arch in ["amd64", "arm64"] {
        let reference = managed_image_reference(
            "ghcr.io/team/repo",
            &format!("linux/{arch}"),
            &format!("sha256:{hash}"),
        )?;
        assert_eq!(reference, format!("ghcr.io/team/repo:ayni-{arch}-{hash}"));
        assert_eq!(reference.rsplit_once(':').ok_or("missing tag")?.1.len(), 75);
    }
    for invalid in ["sha512:abcd", "a", "sha256:bad"] {
        assert!(managed_image_reference("ghcr.io/team/repo", "linux/arm64", invalid).is_err());
    }
    assert!(managed_image_reference("ghcr.io/team/repo", "linux/riscv64", &hash).is_err());
    let path = format!("team/{}", "a".repeat(250));
    assert!(managed_image_reference(&format!("ghcr.io/{path}"), "linux/arm64", &hash).is_ok());
    assert!(managed_image_reference(&format!("ghcr.io/{path}a"), "linux/arm64", &hash).is_err());
    let oversized = format!("{}.io/team/repo", "a".repeat(440));
    assert!(managed_image_reference(&oversized, "linux/arm64", &hash).is_err());
    Ok(())
}

#[test]
fn tls_without_explicit_engine_isolation_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let mut document = operator_document();
    document["profiles"]["project-profile"]["build"]["isolated_engine"] = false.into();
    let configuration: RuntimeConfiguration = serde_json::from_value(document.clone())?;
    assert_eq!(
        configuration.validate(),
        Err("runtime_build_configuration_invalid")
    );
    document["profiles"]["project-profile"]["build"]
        .as_object_mut()
        .ok_or("build")?
        .remove("isolated_engine");
    let configuration: RuntimeConfiguration = serde_json::from_value(document)?;
    assert_eq!(
        configuration.validate(),
        Err("runtime_build_configuration_invalid")
    );
    Ok(())
}

#[test]
fn infrastructure_only_managed_engine_does_not_require_repository_credentials()
-> Result<(), Box<dyn std::error::Error>> {
    let mut document = serde_json::json!({
        "namespace":"ayni-orchestrator-dev", "worker_image":format!("docker.io/local/ayni-orchestrator-worker@sha256:{}", "a".repeat(64)),
        "output_claim":"runtime-outputs", "model_secret":"openai-api", "engine_mode":"local-development",
        "managed_runtime":{"platform":"linux/arm64", "service_account":"runtime-coding", "pull_secret":"runtime-pull",
          "build":{"service_account":"runtime-builder", "engine":{"image":format!("docker.io/library/docker:28.3.3-dind@sha256:{}", "b".repeat(64)), "security_mode":"local-development"}}}
    });
    let invalid: RuntimeConfiguration = serde_json::from_value(document.clone())?;
    assert_eq!(
        invalid.validate(),
        Err("runtime_engine_configuration_invalid")
    );
    document["managed_runtime"]["build"]["engine"]["image"] =
        format!("docker.io/library/docker@sha256:{}", "b".repeat(64)).into();
    let configuration: RuntimeConfiguration = serde_json::from_value(document)?;
    configuration.validate()?;
    Ok(())
}
