//! Durable image installation and validation through the shared image pipeline.
use crate::{
    AppState, coordinators::build_service::runtime,
    process::runtime_configuration::RuntimeConfiguration,
};
use ayni_orchestrator_api::v1;
use ayni_orchestrator_application::{
    execution::{ExecutionBackend, ImageDiscovery},
    execution_settings::WorkloadRuntime,
};
use ayni_orchestrator_kubernetes::execution::{ExecutionState, RuntimeKubernetes};
use ayni_orchestrator_profile_ayni::runtime_execution::ExecutionRequest;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use prost::Message;
use serde_json::{Value, json};

fn decode<M: Message + Default>(cursor: &Value, field: &str) -> Result<M, &'static str> {
    M::decode(
        STANDARD
            .decode(cursor[field].as_str().ok_or("build_test_state_invalid")?)
            .map_err(|_| "build_test_state_invalid")?
            .as_slice(),
    )
    .map_err(|_| "build_test_state_invalid")
}
impl AppState {
    /// Advance one durable service test and retire its owned resources.
    /// # Errors
    /// Reports unavailable persistence, invalid configuration, or Kubernetes failures.
    pub async fn reconcile_build_service(&self) -> Result<(), &'static str> {
        let settings = self
            .database
            .execute(|s| s.build_settings())
            .await
            .map_err(|_| "build_settings_unavailable")?;
        if !matches!(settings.status.as_str(), "testing" | "installing") {
            return Ok(());
        }
        let id = settings.test_id.clone();
        let read_id = id.clone();
        let mut cursor = self
            .database
            .execute(move |s| s.build_test_cursor(&read_id))
            .await
            .map_err(|_| "build_settings_unavailable")?;
        let mut frozen: v1::BuildServiceSettings = decode(&cursor, "settings")?;
        let registry: v1::Connection = decode(&cursor, "registry")?;
        let certificate: v1::Certificate = decode(&cursor, "certificate")?;
        let config =
            RuntimeConfiguration::from_environment()?.ok_or("runtime_configuration_unavailable")?;
        let kube = RuntimeKubernetes::new(config.execution_configuration())?;
        check_frozen_operation(&frozen, &config.engine_mode, &mut cursor);
        if cursor["stage"] == "packaged-cleanup" {
            prepare_fixture(&kube, &mut frozen, &mut cursor).await?;
        } else if cursor["stage"] == "retry-engine" {
            let attempt = attempt_name(&frozen, &cursor);
            if cleanup_attempt(&kube, &attempt).await.is_ok() {
                cursor["attempt"] = json!(cursor["attempt"].as_u64().unwrap_or(0) + 1);
                cursor["stage"] = json!("build");
                cursor["engine_started"] = json!(false);
                cursor
                    .as_object_mut()
                    .ok_or("build_test_state_invalid")?
                    .remove("image_id");
                cursor
                    .as_object_mut()
                    .ok_or("build_test_state_invalid")?
                    .remove("result");
            }
        } else if !matches!(
            cursor["stage"].as_str(),
            Some("cleanup" | "ready" | "failed" | "cancelled")
        ) {
            let stage = cursor["stage"].as_str().unwrap_or("invalid").to_owned();
            let outcome = self
                .advance_image_build(&kube, &frozen, &registry, &certificate, &mut cursor)
                .await;
            match outcome {
                Ok(false) => {}
                Ok(true) => {
                    cursor["outcome"] = json!("ready");
                    cursor["stage"] = json!("cleanup");
                }
                Err("engine_lost") if cursor["retries"].as_u64().unwrap_or(0) < 2 => {
                    cursor["retries"] = json!(cursor["retries"].as_u64().unwrap_or(0) + 1);
                    crate::process::metrics::record_builder_failure("engine", "engine_lost");
                    // Persist retirement before cleanup so restart cannot recreate this engine.
                    cursor["stage"] = json!("retry-engine");
                }
                Err(reason) => {
                    crate::process::metrics::record_builder_failure(stage.as_str(), reason);
                    cursor["outcome"] = json!("failed");
                    cursor["stage"] = json!("cleanup");
                    cursor["reason"] = json!(reason);
                }
            }
        }
        if cursor["stage"] == "cleanup" {
            finish_cleanup(&kube, &frozen, &mut cursor).await;
        }

        let stage = cursor["stage"].as_str().unwrap_or("failed").to_owned();
        self.database
            .execute(move |s| {
                let saved = s.build_test_cursor(&id)?;
                if saved["outcome"] == "cancelled" && cursor["outcome"] != "cancelled" {
                    return Ok(());
                }
                s.save_build_test_cursor(&id, &cursor)?;
                let mut current = s.build_settings()?;
                let finished = apply_test_outcome(&mut current, &frozen, &id, &stage, &cursor);
                let changed =
                    sync_installation(&mut current, &frozen, &id, &stage, &cursor, finished);
                if finished || changed {
                    s.save_build_settings(&current)?;
                }
                Ok(())
            })
            .await
            .map_err(|_| "build_settings_unavailable")
    }
    async fn advance_image_build(
        &self,
        kube: &RuntimeKubernetes,
        s: &v1::BuildServiceSettings,
        registry: &v1::Connection,
        certificate: &v1::Certificate,
        cursor: &mut Value,
    ) -> Result<bool, &'static str> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "build_test_clock_invalid")?
            .as_secs();
        if now.saturating_sub(
            cursor["started"]
                .as_u64()
                .ok_or("build_test_state_invalid")?,
        ) > 3600
        {
            return Err("build_test_timeout");
        }
        let packaging = cursor["producer"] == "builder";
        let workload = image_workload(s, registry, certificate, cursor)?;
        let attempt = attempt_name(s, cursor);
        let name = &attempt;
        if matches!(cursor["stage"].as_str(), Some("build" | "publish"))
            && cursor["engine_started"].as_bool().unwrap_or(false)
            && !kube.engine_alive(name, &workload).await?
        {
            return Err("engine_lost");
        }
        match cursor["stage"].as_str().ok_or("build_test_state_invalid")? {
            "build" => {
                advance_build_stage(kube, name, &workload, s, certificate, cursor, packaging)
                    .await?;
            }
            "publish" => {
                Self::publish_image_stage(kube, &workload, name, cursor, packaging).await?;
            }
            "discover" => {
                let discovery = format!("{name}-discover");
                kube.start_discovery(&discovery, &workload).await?;
                match ayni_orchestrator_application::image_build::publication_readback(
                    kube.discover(&discovery).await?,
                    cursor["image_id"].as_str(),
                    false,
                )? {
                    ImageDiscovery::Pending => {}
                    ImageDiscovery::Resolved { digest, .. } => {
                        cursor["digest"] = json!(digest);
                        cursor["stage"] = json!("verify");
                    }
                    _ => return Err("runtime_publication_readback_failed"),
                }
            }
            "verify" if packaging => verify_packaged_builder(kube, name, cursor, workload).await?,
            "verify" => return Self::verify_test(kube, s, certificate, cursor, workload).await,
            _ => return Err("build_test_state_invalid"),
        }
        Ok(false)
    }
    async fn publish_image_stage(
        kube: &RuntimeKubernetes,
        workload: &WorkloadRuntime,
        name: &str,
        cursor: &mut Value,
        overwrite: bool,
    ) -> Result<(), &'static str> {
        let publish = format!("{name}-publish");
        match kube.coding_state(&publish).await? {
            ExecutionState::Missing => {
                kube.publish_image(
                    &publish,
                    &ayni_orchestrator_kubernetes::engine::bind(workload, name),
                    cursor["image_id"]
                        .as_str()
                        .ok_or("build_test_state_invalid")?,
                    overwrite,
                )
                .await?;
            }
            ExecutionState::Running => {}
            ExecutionState::Failed => {
                let reason = kube.operation_failure(&publish, "publish").await?;
                return Err(match reason.as_str() {
                    "runtime_candidate_missing" => "engine_lost",
                    _ => "runtime_publication_failed",
                });
            }
            ExecutionState::Succeeded => cursor["stage"] = json!("discover"),
        }
        Ok(())
    }
    async fn verify_test(
        kube: &RuntimeKubernetes,
        s: &v1::BuildServiceSettings,
        certificate: &v1::Certificate,
        cursor: &Value,
        mut workload: WorkloadRuntime,
    ) -> Result<bool, &'static str> {
        let attempt = attempt_name(s, cursor);
        let name = &attempt;

        let verify = format!("{name}-verify");
        let request: ExecutionRequest = serde_json::from_value(cursor["result"]["request"].clone())
            .map_err(|_| "build_test_state_invalid")?;
        workload.image = format!(
            "{}@{}",
            workload
                .image
                .rsplit_once(':')
                .ok_or("build_test_state_invalid")?
                .0,
            cursor["digest"]
                .as_str()
                .ok_or("build_test_state_invalid")?
        );
        workload.build = None;
        match kube.coding_state(&verify).await? {
            ExecutionState::Missing => {
                kube.start_builder_verification(
                    &verify,
                    &workload,
                    &request,
                    cursor["result"]["lock"]
                        .as_str()
                        .ok_or("build_test_state_invalid")?,
                    &certificate.key_id,
                    &certificate.public_key,
                )
                .await?;
            }
            ExecutionState::Running => {}
            ExecutionState::Failed => return Err("runtime_verification_failed"),
            ExecutionState::Succeeded => return Ok(true),
        }

        Ok(false)
    }
}

async fn advance_build_stage(
    kube: &RuntimeKubernetes,
    name: &str,
    workload: &WorkloadRuntime,
    settings: &v1::BuildServiceSettings,
    certificate: &v1::Certificate,
    cursor: &mut Value,
    packaging: bool,
) -> Result<(), &'static str> {
    match ayni_orchestrator_application::image_build::observe_build(
        kube,
        name,
        workload,
        cursor["engine_started"].as_bool().unwrap_or(false),
    )
    .await?
    {
        ayni_orchestrator_application::image_build::BuildObservation::Missing => {
            let kind = image_producer(settings, certificate, packaging)?;
            kube.start_image_operation(
                name,
                workload,
                &kind,
                std::collections::BTreeMap::from([("execution.json".into(), "{}".into())]),
                "",
            )
            .await?;
            cursor["engine_started"] = json!(true);
        }
        ayni_orchestrator_application::image_build::BuildObservation::Running => {
            cursor["engine_started"] = json!(true);
        }
        ayni_orchestrator_application::image_build::BuildObservation::Failed(reason) => {
            return Err(build_failure_reason(&reason));
        }
        ayni_orchestrator_application::image_build::BuildObservation::Completed(image_id) => {
            cursor["engine_started"] = json!(true);
            cursor["image_id"] = json!(image_id);
            if !packaging {
                cursor["result"] = kube.builder_test_result(name).await?;
            }
            cursor["stage"] = json!("publish");
        }
        ayni_orchestrator_application::image_build::BuildObservation::EngineLost => {
            return Err("engine_lost");
        }
    }
    Ok(())
}

fn image_producer(
    s: &v1::BuildServiceSettings,
    certificate: &v1::Certificate,
    packaging: bool,
) -> Result<ayni_orchestrator_application::image_build::ImageBuildKind, &'static str> {
    Ok(if packaging {
        ayni_orchestrator_application::image_build::ImageBuildKind::TrustedBuilder {
            recipe_identity: s
                .installation
                .as_ref()
                .ok_or("builder_installation_missing")?
                .recipe_identity
                .clone(),
        }
    } else {
        ayni_orchestrator_application::image_build::ImageBuildKind::TrustedFixture {
            provisioning_base: s.provisioning_base.clone(),
            key_id: certificate.key_id.clone(),
            public_key: certificate.public_key.clone(),
        }
    })
}

fn check_frozen_operation(frozen: &v1::BuildServiceSettings, mode: &str, cursor: &mut Value) {
    if cursor["outcome"] == "cancelled" {
        return;
    }
    let reason = if frozen.security_mode != mode {
        Some("engine_deployment_mode_mismatch")
    } else if frozen.installation.as_ref().is_some_and(|i| {
        i.id == frozen.test_id
            && i.recipe_identity
                != ayni_orchestrator_profile_ayni::ayni_release::builder_recipe_identity()
    }) {
        Some("builder_recipe_changed")
    } else {
        None
    };
    if let Some(reason) = reason {
        cursor["stage"] = json!("cleanup");
        cursor["outcome"] = json!("failed");
        cursor["reason"] = json!(reason);
    }
}

async fn prepare_fixture(
    kube: &RuntimeKubernetes,
    frozen: &mut v1::BuildServiceSettings,
    cursor: &mut Value,
) -> Result<(), &'static str> {
    let attempt = attempt_name(frozen, cursor);
    if cleanup_attempt(kube, &attempt).await.is_ok() {
        frozen.builder_image = cursor["builder_digest"]
            .as_str()
            .ok_or("builder_digest_missing")?
            .into();
        cursor["settings"] = json!(STANDARD.encode(frozen.encode_to_vec()));
        cursor["producer"] = json!("fixture");
        cursor["attempt"] = json!(cursor["attempt"].as_u64().unwrap_or(0) + 1);
        cursor["stage"] = json!("build");
        cursor["engine_started"] = json!(false);
        for field in ["image_id", "result", "digest"] {
            cursor
                .as_object_mut()
                .ok_or("build_test_state_invalid")?
                .remove(field);
        }
    }
    Ok(())
}
fn image_workload(
    s: &v1::BuildServiceSettings,
    registry: &v1::Connection,
    certificate: &v1::Certificate,
    cursor: &Value,
) -> Result<WorkloadRuntime, &'static str> {
    let packaging = cursor["producer"] == "builder";
    let destination = format!(
        "ghcr.io/{}/orchestrator:builder-{}",
        registry.image_namespace,
        attempt_name(s, cursor).trim_start_matches("runtime-")
    );
    let mut workload = WorkloadRuntime {
        image: if packaging {
            s.installation
                .as_ref()
                .ok_or("builder_installation_missing")?
                .image_tag
                .clone()
        } else {
            destination
        },
        platform: s.platform.clone(),
        service_account: "runtime-coding".into(),
        pull_secret: Some(registry.pull_secret_name.clone()),
        build: Some(runtime(
            s,
            registry.secret_name.clone(),
            certificate.secret_name.clone(),
        )),
    };
    if packaging && let Some(build) = &mut workload.build {
        build.image = ayni_orchestrator_profile_ayni::ayni_release::DOCKER_CLIENT_IMAGE.into();
        build.signing_secret.clear();
    }
    Ok(workload)
}
async fn verify_packaged_builder(
    kube: &RuntimeKubernetes,
    name: &str,
    cursor: &mut Value,
    mut workload: WorkloadRuntime,
) -> Result<(), &'static str> {
    let verify = format!("{name}-verify");
    let repository =
        ayni_orchestrator_application::execution_settings::image_repository(&workload.image)
            .ok_or("runtime_image_invalid")?;
    workload.image = format!(
        "{repository}@{}",
        cursor["digest"].as_str().ok_or("builder_digest_missing")?
    );
    workload.build = None;
    match kube.coding_state(&verify).await? {
        ExecutionState::Missing => kube.start_builder_probe(&verify, &workload).await?,
        ExecutionState::Running => {}
        ExecutionState::Failed => return Err("builder_probe_failed"),
        ExecutionState::Succeeded => {
            cursor["builder_digest"] = json!(workload.image);
            cursor["stage"] = json!("packaged-cleanup");
        }
    }
    Ok(())
}
fn sync_installation(
    current: &mut v1::BuildServiceSettings,
    frozen: &v1::BuildServiceSettings,
    id: &str,
    stage: &str,
    cursor: &Value,
    finished: bool,
) -> bool {
    if current.resource_version != frozen.resource_version || current.test_id != id {
        return false;
    }
    let Some(installation) = &mut current.installation else {
        return false;
    };
    if installation.id != id {
        return false;
    }
    installation.stage = format!(
        "{}-{stage}",
        cursor["producer"].as_str().unwrap_or("fixture")
    );
    installation.retry_count = u32::try_from(cursor["retries"].as_u64().unwrap_or(0)).unwrap_or(2);
    installation.image_digest = cursor["builder_digest"].as_str().unwrap_or_default().into();
    if finished {
        installation.status.clone_from(&current.status);
        installation
            .failure_reason
            .clone_from(&current.failure_reason);
    }
    true
}

fn build_failure_reason(reason: &str) -> &'static str {
    match reason {
        "runtime_version_unsupported" => "runtime_version_unsupported",
        "engine_tls_or_startup_failed" => "engine_tls_or_startup_failed",
        _ => "runtime_build_failed",
    }
}

fn attempt_name(settings: &v1::BuildServiceSettings, cursor: &Value) -> String {
    match cursor["attempt"].as_u64().unwrap_or(0) {
        0 => settings.test_id.clone(),
        attempt => {
            ayni_orchestrator_kubernetes::execution::operation_name(&settings.test_id, attempt)
        }
    }
}
async fn cleanup_attempt(kube: &RuntimeKubernetes, attempt: &str) -> Result<(), &'static str> {
    // Stop clients before retiring their engine and TLS keys.
    for name in [
        format!("{attempt}-publish"),
        format!("{attempt}-discover"),
        format!("{attempt}-verify"),
        attempt.into(),
    ] {
        kube.cleanup(&name).await?;
    }
    Ok(())
}

async fn finish_cleanup(
    kube: &RuntimeKubernetes,
    settings: &v1::BuildServiceSettings,
    cursor: &mut Value,
) {
    let attempt = attempt_name(settings, cursor);
    if cleanup_attempt(kube, &attempt).await.is_err() {
        crate::process::metrics::record_builder_failure("cleanup", "engine_cleanup_failed");
        return;
    }
    cursor["stage"] = cursor["outcome"].clone();
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .saturating_sub(cursor["started"].as_u64().unwrap_or_default());
    crate::process::metrics::record_builder_duration(elapsed);
}

fn apply_test_outcome(
    current: &mut v1::BuildServiceSettings,
    frozen: &v1::BuildServiceSettings,
    id: &str,
    stage: &str,
    cursor: &Value,
) -> bool {
    if current.resource_version != frozen.resource_version || current.test_id != id {
        return false;
    }
    match stage {
        "ready" => {
            if let Some(image) = cursor["builder_digest"].as_str() {
                current.builder_image = image.into();
            }
            current.status = "ready".into();
            current.tested_revision = frozen.resource_version;
            current.failure_reason.clear();
        }
        "failed" | "cancelled" => {
            current.status = stage.into();
            current.failure_reason = cursor["reason"]
                .as_str()
                .unwrap_or("build_test_failed")
                .into();
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_image_uses_the_pinned_runtime_contract() {
        assert!(
            ayni_orchestrator_runtime::runtime_contract::RuntimeImage::parse(
                ayni_orchestrator_profile_ayni::ayni_release::DOCKER_CLIENT_IMAGE
            )
            .is_ok()
        );
    }

    #[test]
    fn completed_installation_records_digest_and_preserves_terminal_progress_on_restart() {
        let frozen = v1::BuildServiceSettings {
            resource_version: 2,
            test_id: "install".into(),
            installation: Some(v1::BuilderInstallation {
                id: "install".into(),
                status: "running".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut current = frozen.clone();
        current.status = "installing".into();
        let cursor = json!({"producer":"fixture","stage":"ready","outcome":"ready","builder_digest":"registry.test/builder@sha256:abc"});
        assert!(apply_test_outcome(
            &mut current,
            &frozen,
            "install",
            "ready",
            &cursor
        ));
        assert!(sync_installation(
            &mut current,
            &frozen,
            "install",
            "ready",
            &cursor,
            true
        ));
        assert_eq!(
            current.builder_image,
            cursor["builder_digest"].as_str().unwrap_or_default()
        );
        assert_eq!(
            current.installation.as_ref().map(|i| i.status.as_str()),
            Some("ready")
        );
        assert_eq!(current.tested_revision, 2);
        let previous = current.builder_image.clone();
        assert!(apply_test_outcome(
            &mut current,
            &frozen,
            "install",
            "failed",
            &json!({"reason":"verification_failed","builder_digest":"wrong"})
        ));
        assert_eq!(current.builder_image, previous);
        assert_eq!(current.status, "failed");
    }
    #[test]
    fn deployment_and_recipe_changes_invalidate_completion_but_preserve_cancellation() {
        let frozen = v1::BuildServiceSettings {
            security_mode: "dedicated-builder".into(),
            test_id: "install".into(),
            installation: Some(v1::BuilderInstallation {
                id: "install".into(),
                recipe_identity: "old".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut cursor = json!({"stage":"ready","outcome":"ready"});
        check_frozen_operation(&frozen, "dedicated-builder", &mut cursor);
        assert_eq!(cursor["stage"], "cleanup");
        assert_eq!(cursor["reason"], "builder_recipe_changed");
        check_frozen_operation(&frozen, "local-development", &mut cursor);
        assert_eq!(cursor["reason"], "engine_deployment_mode_mismatch");
        cursor = json!({"stage":"cleanup","outcome":"cancelled","reason":"cancelled"});
        check_frozen_operation(&frozen, "local-development", &mut cursor);
        assert_eq!(cursor["outcome"], "cancelled");
        assert_eq!(cursor["reason"], "cancelled");
    }
    #[test]
    fn stale_revision_or_replaced_test_cannot_activate_settings() {
        let frozen = v1::BuildServiceSettings {
            resource_version: 3,
            test_id: "old".into(),
            ..Default::default()
        };
        let mut current = v1::BuildServiceSettings {
            resource_version: 4,
            test_id: "old".into(),
            status: "configured".into(),
            ..Default::default()
        };
        assert!(!apply_test_outcome(
            &mut current,
            &frozen,
            "old",
            "ready",
            &json!({})
        ));
        assert_eq!(current.status, "configured");
        current.resource_version = 3;
        current.test_id = "new".into();
        assert!(!apply_test_outcome(
            &mut current,
            &frozen,
            "old",
            "ready",
            &json!({})
        ));
        assert_eq!(current.tested_revision, 0);
        current.test_id = "old".into();
        assert!(apply_test_outcome(
            &mut current,
            &frozen,
            "old",
            "ready",
            &json!({})
        ));
        assert_eq!(current.tested_revision, 3);
    }
}
