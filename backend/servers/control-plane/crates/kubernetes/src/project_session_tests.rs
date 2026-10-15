use super::*;
fn pod() -> Result<Pod, &'static str> {
    session_pod(
        "session-test",
        "session-id",
        1,
        "worker@sha256:fixture",
        "model",
        "model-uid",
        "revision",
    )
}
#[test]
fn resident_pod_separates_source_model_results_and_workspace_authority() -> Result<(), &'static str>
{
    let pod = pod()?;
    let spec = pod.spec.as_ref().ok_or("spec")?;
    assert_eq!(spec.restart_policy.as_deref(), Some("Never"));
    assert_eq!(spec.active_deadline_seconds, None);
    assert_eq!(spec.automount_service_account_token, Some(false));
    let checkout = &spec.containers[0];
    let research = &spec.containers[1];
    let mounts = |container: &k8s_openapi::api::core::v1::Container| {
        container.volume_mounts.clone().unwrap_or_default()
    };
    assert!(!mounts(checkout).iter().any(|m| m.name == "model"));
    assert!(
        !mounts(research)
            .iter()
            .any(|m| m.name == "checkout-scratch")
    );
    assert!(
        mounts(checkout)
            .iter()
            .any(|m| m.name == "results" && m.read_only == Some(true))
    );
    assert!(
        mounts(research)
            .iter()
            .any(|m| m.name == "workspace" && m.read_only == Some(true))
    );
    assert!(
        mounts(research)
            .iter()
            .any(|m| m.name == "model" && m.read_only == Some(true))
    );
    assert!(
        !serde_json::to_string(&pod)
            .map_err(|_| "json")?
            .contains("source_tokens")
    );
    validate_pod_spec(&pod)?;
    Ok(())
}
#[test]
fn unexpected_container_privileges_or_scope_cannot_be_adopted() -> Result<(), &'static str> {
    let original = pod()?;
    let mut changed = original.clone();
    changed.spec.as_mut().ok_or("spec")?.host_network = Some(true);
    assert!(!spec_matches(&changed, &original));
    let mut changed = original.clone();
    changed.spec.as_mut().ok_or("spec")?.containers[1].image = Some("other-image".into());
    assert!(validate_pod_spec(&changed).is_err());
    let mut changed = original.clone();
    changed.spec.as_mut().ok_or("spec")?.containers[1]
        .volume_mounts
        .as_mut()
        .ok_or("mounts")?[0]
        .read_only = Some(false);
    assert!(!spec_matches(&changed, &original));
    let mut changed = original;
    changed
        .metadata
        .annotations
        .as_mut()
        .ok_or("annotations")?
        .insert("ayni.io/session-id".into(), "other-session".into());
    assert!(validate_pod_spec(&changed).is_err());
    Ok(())
}

#[test]
fn idle_worker_upgrade_recreates_without_retargeting_an_active_turn() -> Result<(), &'static str> {
    let original = pod()?;
    assert!(idle_compatible(&original, "worker@sha256:fixture"));
    assert!(!idle_compatible(&original, "worker@sha256:replacement"));
    // Active collection checks its recorded UID and original immutable spec.
    validate_pod_spec(&original)?;
    Ok(())
}

#[test]
fn websocket_exec_uses_get_permission_in_the_runtime_role() -> Result<(), &'static str> {
    let request = kube::core::Request::new("/api/v1/namespaces/test/pods")
        .exec(
            "planner",
            [WORKER, "project-session-result"],
            &AttachParams::default().container("research").stdin(true),
        )
        .map_err(|_| "exec request invalid")?;
    assert_eq!(request.method().as_str(), "GET");
    Ok(())
}

#[test]
fn extra_execution_credentials_security_and_volume_sources_are_rejected() -> Result<(), &'static str>
{
    let original = pod()?;
    let source = serde_json::to_value(&original).map_err(|_| "json")?;
    for (pointer, key, value) in [
        ("/spec/containers/0", "command", json!(["/bin/sh"])),
        (
            "/spec/containers/1",
            "envFrom",
            json!([{"secretRef":{"name":"source"}}]),
        ),
        ("/spec/containers/0", "workingDir", json!("/workspace")),
        (
            "/spec/containers/0",
            "lifecycle",
            json!({"postStart":{"exec":{"command":["sh"]}}}),
        ),
        (
            "/spec/containers/0/securityContext",
            "privileged",
            json!(true),
        ),
        ("/spec/volumes/0", "hostPath", json!({"path":"/"})),
    ] {
        let mut changed = source.clone();
        changed
            .pointer_mut(pointer)
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("object")?
            .insert(key.into(), value);
        let changed: Pod = serde_json::from_value(changed).map_err(|_| "pod")?;
        assert!(!spec_matches(&changed, &original), "unexpected {key}");
        assert!(validate_pod_spec(&changed).is_err());
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires a disposable Docker Desktop namespace; server dry-run creates no Pod"]
async fn server_defaulted_session_spec_remains_adoptable() -> Result<(), Box<dyn std::error::Error>>
{
    let namespace = std::env::var("AYNI_SESSION_TEST_NAMESPACE")?;
    if !namespace.starts_with("ayni-sessions-") {
        return Err(std::io::Error::other("disposable namespace required").into());
    }
    let config = kube::Config::from_kubeconfig(&kube::config::KubeConfigOptions {
        context: Some("docker-desktop".into()),
        ..Default::default()
    })
    .await?;
    let client = kube::Client::try_from(config)?;
    let pods: Api<Pod> = Api::namespaced(client, &namespace);
    let expected = pod().map_err(std::io::Error::other)?;
    let actual = pods
        .create(
            &PostParams {
                dry_run: true,
                ..Default::default()
            },
            &expected,
        )
        .await?;
    assert!(
        spec_matches(&actual, &expected),
        "server spec: {:?}",
        actual.spec
    );
    validate_pod_spec(&actual).map_err(std::io::Error::other)?;
    Ok(())
}
