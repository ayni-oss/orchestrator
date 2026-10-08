//! Global operator settings and bounded stable-release discovery.
use crate::{
    AppState, adapters::sqlite::CleanStoreError, coordinators::with_store_async,
    process::runtime_configuration::RuntimeConfiguration,
};
use ayni_orchestrator_api::v1;
use ayni_orchestrator_application::execution_settings::{BuildRuntime, EngineRuntime};
use ayni_orchestrator_profile_ayni::ayni_release;
use ayni_orchestrator_runtime::runtime_contract::{RuntimeImage, RuntimePlatform};
use prost::Message;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::OnceLock,
    time::{Duration, Instant},
};
use tonic::Status;

fn status(error: &CleanStoreError) -> Status {
    match error {
        CleanStoreError::StaleVersion => Status::aborted("build_settings_conflict"),
        CleanStoreError::IdempotencyConflict => Status::already_exists("idempotency_conflict"),
        CleanStoreError::InvalidTransition => {
            Status::failed_precondition("build_settings_precondition")
        }
        _ => Status::internal("build_settings_unavailable"),
    }
}
fn operation(kind: &str, key: &str, request: &impl Message) -> Result<String, Status> {
    if key.is_empty()
        || key.len() > 128
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(Status::invalid_argument("idempotency_key_invalid"));
    }
    Ok(format!(
        "{kind}:{:x}",
        Sha256::digest(request.encode_to_vec())
    ))
}

/// Build scoped runtime authority from safe global settings.
#[must_use]
pub fn runtime(
    settings: &v1::BuildServiceSettings,
    registry_secret: String,
    signing_secret: String,
) -> BuildRuntime {
    BuildRuntime {
        configuration_revision: settings.resource_version,
        image: settings.builder_image.clone(),
        service_account: "runtime-builder".into(),
        registry_secret,
        signing_secret,
        engine_secret: settings.engine_secret.clone(),
        engine_host: settings.engine_host.clone(),
        isolated_engine: settings.security_mode == "external-isolated",
        engine: if settings.security_mode == "external-isolated" {
            None
        } else {
            Some(EngineRuntime {
                image: settings.engine_image.clone(),
                security_mode: settings.security_mode.clone(),
            })
        },
    }
}
fn validate(s: &v1::BuildServiceSettings) -> Result<(), Status> {
    if s.ayni_version != ayni_release::CLI_VERSION {
        return Err(Status::invalid_argument("runtime_version_unsupported"));
    }
    if RuntimeImage::parse(&s.provisioning_base).is_err()
        || RuntimePlatform::parse(&s.platform).is_err()
    {
        return Err(Status::invalid_argument("build_image_or_platform_invalid"));
    }
    if !matches!(
        s.security_mode.as_str(),
        "dedicated-builder" | "local-development" | "external-isolated"
    ) {
        return Err(Status::invalid_argument("engine_security_mode_invalid"));
    }
    if s.security_mode != "external-isolated" && RuntimeImage::parse(&s.engine_image).is_err() {
        return Err(Status::invalid_argument("engine_image_invalid"));
    }
    if s.test_registry_id.is_empty() || s.test_certificate_id.is_empty() {
        return Err(Status::invalid_argument("build_test_connections_required"));
    }
    if s.encode_to_vec().len() > 8192 {
        return Err(Status::invalid_argument("build_settings_oversized"));
    }
    Ok(())
}
fn deployed_mode(config: Option<&RuntimeConfiguration>) -> &str {
    config.map_or("external-isolated", |c| c.engine_mode.as_str())
}

fn apply_deployment(
    desired: &mut v1::BuildServiceSettings,
    config: Option<&RuntimeConfiguration>,
) -> Result<(), Status> {
    let mode = deployed_mode(config);
    if !desired.security_mode.is_empty() && desired.security_mode != mode {
        return Err(Status::failed_precondition(
            "engine_deployment_mode_mismatch",
        ));
    }
    desired.security_mode = mode.into();
    // Endpoint authority is also deployment-owned, never accepted from a client.
    desired.engine_host.clear();
    desired.engine_secret.clear();
    if mode == "external-isolated" {
        let build = config
            .and_then(|c| c.managed_runtime.as_ref())
            .and_then(|c| c.build.as_ref());
        if let Some(build) = build {
            desired.engine_host.clone_from(&build.engine_host);
            desired.engine_secret.clone_from(&build.engine_secret);
        }
    }
    Ok(())
}

impl AppState {
    pub(crate) async fn build_settings(&self) -> Result<v1::BuildServiceSettings, Status> {
        let config = RuntimeConfiguration::from_environment()
            .map_err(|_| Status::unavailable("runtime_configuration_unavailable"))?;
        with_store_async(self, move |store| {
            let mut value = store.build_settings()?;
            if value.resource_version == 0 {
                value.ayni_version = ayni_release::CLI_VERSION.into();
                value.platform = config
                    .as_ref()
                    .and_then(|c| c.managed_runtime.as_ref())
                    .map_or_else(|| "linux/arm64".into(), |c| c.platform.clone());
                value.security_mode = deployed_mode(config.as_ref()).into();
                value.status = "setup_required".into();
                value.management_source = "ui".into();
                if let Some(defaults) = config.as_ref().and_then(|c| c.builder_defaults.as_ref()) {
                    value.builder_image.clone_from(&defaults.builder_image);
                    value.engine_image.clone_from(&defaults.engine_image);
                    value
                        .provisioning_base
                        .clone_from(&defaults.provisioning_base);
                    value
                        .test_registry_id
                        .clone_from(&defaults.test_registry_id);
                    value
                        .test_certificate_id
                        .clone_from(&defaults.test_certificate_id);
                }
                if let Some(build) = config
                    .clone()
                    .and_then(|c| c.managed_runtime)
                    .and_then(|c| c.build)
                {
                    value.builder_image = build.image;
                    value.engine_host = build.engine_host;
                    value.engine_secret = build.engine_secret;
                    value.security_mode = "external-isolated".into();
                    if let Some(engine) = build.engine {
                        value.security_mode = engine.security_mode;
                        value.engine_image = engine.image;
                    }
                    value.management_source = "administrator".into();
                    value.resource_version = 1;
                    value.status = "configured".into();
                    // Safe default selection is possible only when there is one ready identity.
                    let registries = store
                        .list_connections()?
                        .into_iter()
                        .filter(|c| {
                            c.enabled
                                && c.status == "ready"
                                && c.kind == "ghcr"
                                && !c.pull_secret_name.is_empty()
                        })
                        .collect::<Vec<_>>();
                    if value.test_registry_id.is_empty()
                        && let [registry] = registries.as_slice()
                    {
                        value.test_registry_id.clone_from(&registry.id);
                    }
                    let certificates = store
                        .list_certificates()?
                        .into_iter()
                        .filter(|c| c.enabled && c.status == "ready")
                        .collect::<Vec<_>>();
                    if value.test_certificate_id.is_empty()
                        && let [certificate] = certificates.as_slice()
                    {
                        value.test_certificate_id.clone_from(&certificate.id);
                    }
                    store.save_build_settings(&value)?;
                }
            }
            let mode = deployed_mode(config.as_ref());
            if value.security_mode != mode
                && !matches!(value.status.as_str(), "testing" | "installing")
            {
                value.security_mode = mode.into();
                value.resource_version = value
                    .resource_version
                    .checked_add(1)
                    .ok_or(CleanStoreError::BoundExceeded)?;
                value.status = "configured".into();
                value.tested_revision = 0;
                value.failure_reason.clear();
                value.test_id.clear();
                store.save_build_settings(&value)?;
            }
            Ok(value)
        })
        .await
        .map_err(|error| status(&error))
    }
    pub(crate) async fn update_build_service(
        &self,
        request: v1::UpdateBuildServiceRequest,
    ) -> Result<v1::BuildServiceSettings, Status> {
        let digest = operation("update", &request.idempotency_key, &request)?;
        let mut desired = request
            .settings
            .clone()
            .ok_or_else(|| Status::invalid_argument("build_settings_missing"))?;
        let config = RuntimeConfiguration::from_environment()
            .map_err(|_| Status::unavailable("runtime_configuration_unavailable"))?;
        apply_deployment(&mut desired, config.as_ref())?;
        validate(&desired)?;
        // Installed identities are observed output, never accepted as UI authority.
        desired.builder_image.clear();
        desired.installation = None;
        with_store_async(self, move |store| {
            let current = store.build_settings()?;
            if store.build_operation(&request.idempotency_key, &digest)? {
                return Ok(current);
            }
            if matches!(current.status.as_str(), "testing" | "installing")
                || current.management_source == "administrator"
            {
                return Err(CleanStoreError::InvalidTransition);
            }
            if request.expected_resource_version != current.resource_version {
                return Err(CleanStoreError::StaleVersion);
            }
            desired.builder_image.clone_from(&current.builder_image);
            desired.installation.clone_from(&current.installation);
            desired.resource_version = current
                .resource_version
                .checked_add(1)
                .ok_or(CleanStoreError::BoundExceeded)?;
            desired.status = "configured".into();
            desired.management_source = "ui".into();
            desired.failure_reason.clear();
            desired.test_id.clear();
            desired.tested_revision = 0;
            store.save_build_operation(&request.idempotency_key, &digest, &desired)?;
            Ok(desired)
        })
        .await
        .map_err(|error| status(&error))
    }
    pub(crate) async fn start_builder_installation(
        &self,
        request: v1::StartBuilderInstallationRequest,
    ) -> Result<v1::BuildServiceSettings, Status> {
        let digest = operation("install", &request.idempotency_key, &request)?;

        let config = RuntimeConfiguration::from_environment()
            .map_err(|_| Status::unavailable("runtime_configuration_unavailable"))?;

        with_store_async(self, move |store| {
            let mut settings = store.build_settings()?;
            if store.build_operation(&request.idempotency_key, &digest)? {
                return Ok(settings);
            }
            if settings.resource_version != request.expected_resource_version {
                return Err(CleanStoreError::StaleVersion);
            }
            if matches!(settings.status.as_str(), "testing" | "installing")
                || settings.security_mode != deployed_mode(config.as_ref())
            {
                return Err(CleanStoreError::InvalidTransition);
            }
            validate(&settings).map_err(|_| CleanStoreError::InvalidTransition)?;
            let registry = store.get_connection(&settings.test_registry_id)?;
            let certificate = store.get_certificate(&settings.test_certificate_id)?;
            validate_test_authority(&registry, &certificate)?;
            settings.resource_version = settings
                .resource_version
                .checked_add(1)
                .ok_or(CleanStoreError::BoundExceeded)?;
            let id = ayni_orchestrator_kubernetes::execution::operation_name(
                &format!("builder-install:{}", request.idempotency_key),
                settings.resource_version,
            );
            let arch = settings
                .platform
                .strip_prefix("linux/")
                .ok_or(CleanStoreError::InvalidTransition)?;
            settings.installation = Some(v1::BuilderInstallation {
                id: id.clone(),
                configuration_revision: settings.resource_version,
                ayni_version: settings.ayni_version.clone(),
                platform: settings.platform.clone(),
                status: "running".into(),
                stage: "build".into(),
                image_tag: format!(
                    "ghcr.io/{}/orchestrator:ayni-builder-{}-{arch}",
                    registry.image_namespace, settings.ayni_version
                ),
                recipe_identity: ayni_release::builder_recipe_identity(),
                ..Default::default()
            });
            settings.test_id.clone_from(&id);
            settings.status = "installing".into();
            settings.failure_reason.clear();
            settings.tested_revision = 0;
            store.save_build_test_cursor(
                &id,
                &frozen_pipeline_cursor(&settings, &registry, &certificate, "builder"),
            )?;
            store.save_build_operation(&request.idempotency_key, &digest, &settings)?;
            Ok(settings)
        })
        .await
        .map_err(|error| status(&error))
    }

    pub(crate) async fn cancel_builder_installation(
        &self,
        request: v1::CancelBuilderInstallationRequest,
    ) -> Result<v1::BuildServiceSettings, Status> {
        let digest = operation("cancel-install", &request.idempotency_key, &request)?;
        with_store_async(self, move |store| {
            let settings = store.build_settings()?;
            if store.build_operation(&request.idempotency_key, &digest)? {
                return Ok(settings);
            }
            if settings.resource_version != request.expected_resource_version {
                return Err(CleanStoreError::StaleVersion);
            }
            if settings.status != "installing" {
                return Err(CleanStoreError::InvalidTransition);
            }
            let mut cursor = store.build_test_cursor(&settings.test_id)?;
            cursor["stage"] = serde_json::json!("cleanup");
            cursor["outcome"] = serde_json::json!("cancelled");
            cursor["reason"] = serde_json::json!("builder_installation_cancelled");
            store.save_build_test_cursor(&settings.test_id, &cursor)?;
            store.save_build_operation(&request.idempotency_key, &digest, &settings)?;
            Ok(settings)
        })
        .await
        .map_err(|error| status(&error))
    }

    pub(crate) async fn cancel_build_service_test(
        &self,
        request: v1::CancelBuildServiceTestRequest,
    ) -> Result<v1::BuildServiceSettings, Status> {
        let digest = operation("cancel", &request.idempotency_key, &request)?;
        with_store_async(self, move |store| {
            let current = store.build_settings()?;
            if store.build_operation(&request.idempotency_key, &digest)? {
                return Ok(current);
            }
            if current.resource_version != request.expected_resource_version {
                return Err(CleanStoreError::StaleVersion);
            }
            if current.status != "testing" {
                return Err(CleanStoreError::InvalidTransition);
            }
            let mut cursor = store.build_test_cursor(&current.test_id)?;
            cursor["stage"] = serde_json::json!("cleanup");
            cursor["outcome"] = serde_json::json!("cancelled");
            cursor["reason"] = serde_json::json!("build_test_cancelled");
            store.save_build_test_cursor(&current.test_id, &cursor)?;
            store.save_build_operation(&request.idempotency_key, &digest, &current)?;
            Ok(current)
        })
        .await
        .map_err(|error| status(&error))
    }
    pub(crate) async fn test_build_service(
        &self,
        request: v1::TestBuildServiceRequest,
    ) -> Result<v1::BuildServiceSettings, Status> {
        let digest = operation("test", &request.idempotency_key, &request)?;

        let config = RuntimeConfiguration::from_environment()
            .map_err(|_| Status::unavailable("runtime_configuration_unavailable"))?;

        with_store_async(self, move |store| {
            let mut settings = store.build_settings()?;
            if settings.security_mode != deployed_mode(config.as_ref()) {
                return Err(CleanStoreError::InvalidTransition);
            }
            if store.build_operation(&request.idempotency_key, &digest)? {
                return Ok(settings);
            }
            if settings.resource_version != request.expected_resource_version {
                return Err(CleanStoreError::StaleVersion);
            }
            if matches!(settings.status.as_str(), "testing" | "installing")
                || RuntimeImage::parse(&settings.builder_image).is_err()
                || settings.installation.as_ref().is_some_and(|installed| {
                    installed.status != "ready"
                        || installed.ayni_version != settings.ayni_version
                        || installed.platform != settings.platform
                        || installed.image_digest != settings.builder_image
                })
            {
                return Err(CleanStoreError::InvalidTransition);
            }
            let registry = store.get_connection(&settings.test_registry_id)?;
            let certificate = store.get_certificate(&settings.test_certificate_id)?;
            validate_test_authority(&registry, &certificate)?;
            settings.test_id = ayni_orchestrator_kubernetes::execution::operation_name(
                &format!("service-test:{}", request.idempotency_key),
                settings.resource_version,
            );
            settings.status = "testing".into();
            settings.failure_reason.clear();
            store.save_build_test_cursor(
                &settings.test_id,
                &frozen_pipeline_cursor(&settings, &registry, &certificate, "fixture"),
            )?;
            store.save_build_operation(&request.idempotency_key, &digest, &settings)?;
            Ok(settings)
        })
        .await
        .map_err(|error| status(&error))
    }
}

fn validate_test_authority(
    registry: &v1::Connection,
    certificate: &v1::Certificate,
) -> Result<(), CleanStoreError> {
    if !registry.enabled
        || registry.status != "ready"
        || registry.kind != "ghcr"
        || registry.pull_secret_name.is_empty()
        || !certificate.enabled
        || certificate.status != "ready"
    {
        return Err(CleanStoreError::InvalidTransition);
    }
    Ok(())
}

fn frozen_pipeline_cursor(
    settings: &v1::BuildServiceSettings,
    registry: &v1::Connection,
    certificate: &v1::Certificate,
    producer: &str,
) -> serde_json::Value {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    serde_json::json!({
        "producer": producer,
        "stage": "build",
        "settings": STANDARD.encode(settings.encode_to_vec()),
        "registry": STANDARD.encode(registry.encode_to_vec()),
        "certificate": STANDARD.encode(certificate.encode_to_vec()),
        "started": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs(),
    })
}

type ReleaseCache = BTreeMap<u32, (Instant, v1::ListAyniReleasesResponse)>;
static RELEASES: OnceLock<tokio::sync::Mutex<ReleaseCache>> = OnceLock::new();
#[derive(serde::Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    published_at: Option<String>,
    draft: bool,
    prerelease: bool,
}
/// Discover bounded stable Ayni release metadata, retaining cached results on outages.
/// # Errors
/// Rejects out-of-range page numbers.
pub async fn releases(page: u32) -> Result<v1::ListAyniReleasesResponse, Status> {
    let page = page.max(1);
    if page > 100 {
        return Err(Status::invalid_argument("release_page_invalid"));
    }
    let cache = RELEASES.get_or_init(|| tokio::sync::Mutex::new(BTreeMap::new()));
    let previous = cache.lock().await.get(&page).cloned();
    if let Some((at, mut value)) = previous.clone()
        && at.elapsed() < Duration::from_secs(300)
    {
        value.cached = true;
        return Ok(value);
    }
    let fetch = async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(8))
            .build()
            .map_err(|_| ())?;
        let response = client
            .get(format!(
                "https://api.github.com/repos/ayni-oss/ayni/releases?per_page=30&page={page}"
            ))
            .header("User-Agent", "ayni-orchestrator")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|_| ())?
            .error_for_status()
            .map_err(|_| ())?;
        if response.content_length().is_some_and(|n| n > 1_048_576) {
            return Err(());
        }
        let bytes = response.bytes().await.map_err(|_| ())?;
        if bytes.len() > 1_048_576 {
            return Err(());
        }
        serde_json::from_slice::<Vec<Release>>(&bytes).map_err(|_| ())
    }
    .await;
    let Ok(releases) = fetch else {
        if let Some((_, mut value)) = previous {
            value.cached = true;
            value.discovery_status = "unavailable_cached".into();
            return Ok(value);
        }
        return Ok(v1::ListAyniReleasesResponse {
            discovery_status: "unavailable".into(),
            supported_version: ayni_release::CLI_VERSION.into(),
            ..Default::default()
        });
    };
    let response = release_response(releases, page);
    cache
        .lock()
        .await
        .insert(page, (Instant::now(), response.clone()));
    Ok(response)
}

fn release_response(releases: Vec<Release>, page: u32) -> v1::ListAyniReleasesResponse {
    let next_page = if releases.len() == 30 { page + 1 } else { 0 };
    let mut values = releases
        .into_iter()
        .filter(|r| !r.draft && !r.prerelease)
        .filter_map(|r| {
            let version = r.tag_name.strip_prefix("ayni-v")?.to_owned();
            if !version
                .split('.')
                .all(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
            {
                return None;
            }
            Some(v1::AyniRelease {
                supported: version == ayni_release::CLI_VERSION,
                version,
                tag: r.tag_name,
                url: r.html_url,
                published_at: r.published_at.unwrap_or_default(),
            })
        })
        .collect::<Vec<_>>();
    values.sort_by(|a, b| b.published_at.cmp(&a.published_at));
    v1::ListAyniReleasesResponse {
        releases: values,
        next_page,
        cached: false,
        discovery_status: "ready".into(),
        supported_version: ayni_release::CLI_VERSION.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn release_discovery_filters_unstable_tags_and_preserves_unsupported_versions()
    -> Result<(), serde_json::Error> {
        let releases = serde_json::from_str(
            r#"[
          {"tag_name":"ayni-v0.14.5","html_url":"https://github.com/ayni-oss/ayni/releases/tag/ayni-v0.14.5","published_at":"2026-10-03","draft":false,"prerelease":false},
          {"tag_name":"ayni-v0.15.0","html_url":"https://github.com/ayni-oss/ayni/releases/tag/ayni-v0.15.0","published_at":"2026-10-04","draft":false,"prerelease":false},
          {"tag_name":"ayni-v0.16.0","html_url":"","published_at":null,"draft":false,"prerelease":true},
          {"tag_name":"ayni-v0.17.0","html_url":"","published_at":null,"draft":true,"prerelease":false},
          {"tag_name":"unrelated","html_url":"","published_at":null,"draft":false,"prerelease":false}
        ]"#,
        )?;
        let result = release_response(releases, 1);
        assert_eq!(result.releases.len(), 2);
        assert_eq!(result.releases[0].version, "0.15.0");
        assert!(!result.releases[0].supported);
        assert!(result.releases[1].supported);
        assert_eq!(result.next_page, 0);
        Ok(())
    }
    #[tokio::test]
    async fn release_pagination_rejects_unbounded_requests() {
        assert!(releases(101).await.is_err());
    }
    fn settings() -> v1::BuildServiceSettings {
        v1::BuildServiceSettings {
            ayni_version: ayni_release::CLI_VERSION.into(),
            builder_image: format!("registry.test/builder@sha256:{}", "a".repeat(64)),
            engine_image: format!("registry.test/docker@sha256:{}", "b".repeat(64)),
            provisioning_base: format!("registry.test/base@sha256:{}", "c".repeat(64)),
            platform: "linux/arm64".into(),
            security_mode: "external-isolated".into(),
            test_registry_id: "registry".into(),
            test_certificate_id: "certificate".into(),
            ..Default::default()
        }
    }
    #[test]
    fn installation_accepts_no_builder_but_requires_supported_version_and_pinned_inputs() {
        let mut value = settings();
        assert!(validate(&value).is_ok());
        value.ayni_version = "0.14.4".into();
        assert!(validate(&value).is_err());
        value.ayni_version = ayni_release::CLI_VERSION.into();
        value.builder_image.clear();
        assert!(validate(&value).is_ok());
        value.provisioning_base = "registry.test/base:latest".into();
        assert!(validate(&value).is_err());
    }
    #[test]
    fn client_cannot_authorize_privileged_engines_or_external_endpoints() {
        let mut desired = settings();
        desired.security_mode = "dedicated-builder".into();
        assert!(apply_deployment(&mut desired, None).is_err());
        desired.security_mode.clear();
        desired.engine_host = "tcp://attacker:2376".into();
        desired.engine_secret = "attacker".into();
        assert!(apply_deployment(&mut desired, None).is_ok());
        assert_eq!(desired.security_mode, "external-isolated");
        assert!(desired.engine_host.is_empty());
        assert!(desired.engine_secret.is_empty());
    }
    #[tokio::test]
    async fn cancellation_is_durable_versioned_and_idempotent()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("state.sqlite");
        let state = AppState::open(&path, 3)?;
        state
            .database
            .execute(|store| {
                store.save_build_settings(&v1::BuildServiceSettings {
                    resource_version: 7,
                    status: "testing".into(),
                    test_id: "fixture".into(),
                    ..Default::default()
                })?;
                store.save_build_test_cursor(
                    "fixture",
                    &serde_json::json!({"stage":"publish","attempt":1,"image_id":"candidate"}),
                )
            })
            .await?;
        let request = v1::CancelBuildServiceTestRequest {
            idempotency_key: "cancel".into(),
            expected_resource_version: 7,
        };
        let first = state.cancel_build_service_test(request.clone()).await?;
        assert_eq!(first.status, "testing"); // readiness waits for cleanup
        assert_eq!(state.cancel_build_service_test(request).await?, first);
        assert!(
            state
                .cancel_build_service_test(v1::CancelBuildServiceTestRequest {
                    idempotency_key: "stale-cancel".into(),
                    expected_resource_version: 6
                })
                .await
                .is_err()
        );
        state.shutdown_database().await?;
        let state = AppState::open(&path, 3)?;
        let cursor = state
            .database
            .execute(|s| s.build_test_cursor("fixture"))
            .await?;
        assert_eq!(cursor["stage"], "cleanup");
        assert_eq!(cursor["outcome"], "cancelled");
        assert_eq!(cursor["attempt"], 1);
        state.shutdown_database().await?;
        Ok(())
    }
    #[tokio::test]
    async fn settings_mutations_are_versioned_idempotent_and_cannot_claim_readiness()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let state = AppState::open(directory.path().join("state.sqlite"), 3)?;
        let request = v1::UpdateBuildServiceRequest {
            idempotency_key: "configuration".into(),
            expected_resource_version: 0,
            settings: Some(v1::BuildServiceSettings {
                status: "ready".into(),

                ..settings()
            }),
        };
        let first = state.update_build_service(request.clone()).await?;
        assert_eq!(first.status, "configured");
        assert_eq!(first.tested_revision, 0);
        assert_eq!(first.resource_version, 1);
        assert_eq!(state.update_build_service(request).await?, first);
        assert!(
            state
                .update_build_service(v1::UpdateBuildServiceRequest {
                    idempotency_key: "stale".into(),
                    expected_resource_version: 0,
                    settings: Some(settings())
                })
                .await
                .is_err()
        );
        state.shutdown_database().await?;
        Ok(())
    }
    #[tokio::test]
    async fn first_installation_is_versioned_idempotent_and_cancellation_survives_restart()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("install.sqlite");
        let state = AppState::open(&path, 3)?;
        state
            .database
            .execute(|store| {
                let mut desired = settings();
                desired.builder_image.clear();
                desired.resource_version = 1;
                desired.status = "configured".into();
                store.save_build_settings(&desired)?;
                store.save_connection(&v1::Connection {
                    id: "registry".into(),
                    kind: "ghcr".into(),
                    enabled: true,
                    status: "ready".into(),
                    resource_version: 1,
                    image_namespace: "ayni-oss".into(),
                    secret_name: "push".into(),
                    pull_secret_name: "pull".into(),
                    ..Default::default()
                })?;
                store.save_certificate(&v1::Certificate {
                    id: "certificate".into(),
                    resource_version: 1,
                    enabled: true,
                    status: "ready".into(),
                    secret_name: "sign".into(),
                    ..Default::default()
                })?;
                Ok(())
            })
            .await?;
        let request = v1::StartBuilderInstallationRequest {
            idempotency_key: "first-install".into(),
            expected_resource_version: 1,
        };
        let result = state.start_builder_installation(request.clone()).await?;
        assert_eq!(result.status, "installing");
        assert!(result.builder_image.is_empty());
        assert_eq!(result.resource_version, 2);
        assert_eq!(
            result
                .installation
                .as_ref()
                .ok_or("installation")?
                .image_tag,
            "ghcr.io/ayni-oss/orchestrator:ayni-builder-0.14.5-arm64"
        );
        assert_eq!(state.start_builder_installation(request).await?, result);
        assert!(
            state
                .start_builder_installation(v1::StartBuilderInstallationRequest {
                    idempotency_key: "conflicting".into(),
                    expected_resource_version: 2
                })
                .await
                .is_err()
        );
        state
            .cancel_builder_installation(v1::CancelBuilderInstallationRequest {
                idempotency_key: "cancel".into(),
                expected_resource_version: 2,
            })
            .await?;
        let cursor = state
            .database
            .execute(move |store| store.build_test_cursor(&result.test_id))
            .await?;
        assert_eq!(cursor["outcome"], "cancelled");
        assert_eq!(cursor["stage"], "cleanup");
        state
            .database
            .execute(|store| {
                let mut current = store.build_settings()?;
                current.builder_image = settings().builder_image;
                store.save_build_settings(&current)
            })
            .await?;
        assert!(
            state
                .test_build_service(v1::TestBuildServiceRequest {
                    expected_resource_version: 2,
                    idempotency_key: "cancelled-install-retest".into(),
                })
                .await
                .is_err()
        );
        drop(state);
        let reopened = AppState::open(&path, 3)?;
        assert_eq!(
            reopened
                .database
                .execute(|store| store.build_settings())
                .await?
                .status,
            "installing"
        );
        Ok(())
    }
}
