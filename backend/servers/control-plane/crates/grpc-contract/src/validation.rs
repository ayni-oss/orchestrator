//! Transport-level validation shared by gRPC adapters and contract tests.
//!
//! These checks enforce public syntax and size bounds. Domain admission still
//! belongs to the control plane (for example, whether an artifact is verified).

use crate::v1;
use ayni_orchestrator_oci::{ImageRepository, ImageTag, Sha256Digest};

/// A stable validation failure suitable for mapping to `ErrorDetail`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationError {
    /// Stable machine-readable reason.
    pub reason: v1::ErrorReason,
    /// Request field that failed validation.
    pub field: &'static str,
}

impl ValidationError {
    const fn new(reason: v1::ErrorReason, field: &'static str) -> Self {
        Self { reason, field }
    }
}

/// Validates a bounded pagination selection.
///
/// # Errors
///
/// Returns the violated public field bound.
pub const fn page(value: Option<&v1::PageRequest>) -> Result<(), ValidationError> {
    let Some(value) = value else {
        return Ok(());
    };
    if value.page_size > 100 || value.page_token.len() > 512 {
        return Err(ValidationError::new(v1::ErrorReason::BoundExceeded, "page"));
    }
    Ok(())
}

/// Validates an optional request enum filter, rejecting unspecified and unknown values.
///
/// # Errors
///
/// Returns `UNKNOWN_ENUM_VALUE` when the supplied number is not accepted.
pub fn enum_filter(
    value: Option<i32>,
    field: &'static str,
    known_values: &[i32],
) -> Result<(), ValidationError> {
    if value.is_some_and(|value| !known_values.contains(&value)) {
        return Err(ValidationError::new(
            v1::ErrorReason::UnknownEnumValue,
            field,
        ));
    }
    Ok(())
}

/// Validates a caller-generated mutation identity.
///
/// # Errors
///
/// Returns the violated identity constraint.
pub fn idempotency_key(value: &str) -> Result<(), ValidationError> {
    if !(1..=128).contains(&value.len())
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
    {
        return Err(ValidationError::new(
            v1::ErrorReason::InvalidIdentifier,
            "idempotency_key",
        ));
    }
    Ok(())
}

/// Validates an opaque resource identifier used by this API.
///
/// # Errors
///
/// Returns the violated identity constraint.
pub fn resource_id(value: &str, field: &'static str) -> Result<(), ValidationError> {
    if !(1..=128).contains(&value.len())
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
    {
        return Err(ValidationError::new(
            v1::ErrorReason::InvalidIdentifier,
            field,
        ));
    }
    Ok(())
}

/// Validates a GitHub-MVP commit identity.
///
/// # Errors
///
/// Returns `INVALID_REPOSITORY_REVISION` for a noncanonical SHA.
pub fn git_revision(value: &str, field: &'static str) -> Result<(), ValidationError> {
    if value.len() != 40
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ValidationError::new(
            v1::ErrorReason::InvalidRepositoryRevision,
            field,
        ));
    }
    Ok(())
}

/// Validates a full Ayni lock fingerprint without a prefix.
///
/// # Errors
///
/// Returns `INVALID_LOCK_FINGERPRINT` for a noncanonical fingerprint.
pub fn lock_fingerprint(value: &str) -> Result<(), ValidationError> {
    if !lower_hex(value, 64) {
        return Err(ValidationError::new(
            v1::ErrorReason::InvalidLockFingerprint,
            "lock_fingerprint",
        ));
    }
    Ok(())
}

/// Validates the immutable OCI digest form supported by v1.
///
/// # Errors
///
/// Returns `INVALID_OCI_DIGEST` for a noncanonical digest.
pub fn oci_digest(value: &str, field: &'static str) -> Result<(), ValidationError> {
    if Sha256Digest::parse(value).is_err() {
        return Err(ValidationError::new(
            v1::ErrorReason::InvalidOciDigest,
            field,
        ));
    }
    Ok(())
}

/// Validates one supported OCI platform.
///
/// # Errors
///
/// Returns `INVALID_PLATFORM` for an absent or malformed platform.
pub fn platform(value: Option<&v1::OciPlatform>) -> Result<(), ValidationError> {
    let Some(value) = value else {
        return Err(ValidationError::new(
            v1::ErrorReason::InvalidPlatform,
            "platform",
        ));
    };
    if !oci_component(&value.operating_system)
        || !oci_component(&value.architecture)
        || value
            .variant
            .as_deref()
            .is_some_and(|item| !oci_component(item))
    {
        return Err(ValidationError::new(
            v1::ErrorReason::InvalidPlatform,
            "platform",
        ));
    }
    Ok(())
}

/// Validates a create-repository request at the server boundary.
///
/// # Errors
///
/// Returns the first violated request constraint.
pub fn create_repository_connection(
    value: &v1::CreateRepositoryConnectionRequest,
) -> Result<(), ValidationError> {
    idempotency_key(&value.idempotency_key)?;
    if !(1..=256).contains(&value.display_name.len()) {
        return Err(ValidationError::new(
            v1::ErrorReason::BoundExceeded,
            "display_name",
        ));
    }
    provider(value.provider.as_ref())?;
    provider_reference(&value.default_branch, "default_branch")
}

/// Validates closed profile configuration and profile-specific identities.
///
/// # Errors
///
/// Returns the first violated request constraint.
pub fn create_runtime_profile(
    value: &v1::CreateRuntimeProfileRequest,
) -> Result<(), ValidationError> {
    idempotency_key(&value.idempotency_key)?;
    resource_id(&value.repository_connection_id, "repository_connection_id")?;
    resource_id(&value.repository_snapshot_id, "repository_snapshot_id")?;
    let configuration = value
        .configuration
        .as_ref()
        .ok_or_else(|| ValidationError::new(v1::ErrorReason::InvalidInput, "configuration"))?;
    match configuration.configuration.as_ref() {
        Some(v1::runtime_profile_configuration::Configuration::AyniManaged(profile)) => {
            if !profile.lock_fingerprint.is_empty() {
                lock_fingerprint(&profile.lock_fingerprint)?;
            }
            if let Some(digest) = &profile.runtime_source_digest {
                oci_digest(digest, "runtime_source_digest")?;
            }
            safe_text(&profile.ayni_version, 1, 128, "ayni_version")?;
            oci_repository(&profile.oci_repository)
        }
        Some(v1::runtime_profile_configuration::Configuration::CustomRuntime(profile)) => {
            oci_reference(&profile.image_reference, "image_reference")
        }
        None => Err(ValidationError::new(
            v1::ErrorReason::UnknownEnumValue,
            "configuration",
        )),
    }
}

/// Validates every caller-controlled field frozen by task admission.
///
/// # Errors
///
/// Returns the first violated request constraint.
pub fn create_development_task(
    value: &v1::CreateDevelopmentTaskRequest,
) -> Result<(), ValidationError> {
    idempotency_key(&value.idempotency_key)?;
    let request = value
        .request
        .as_ref()
        .ok_or_else(|| ValidationError::new(v1::ErrorReason::InvalidInput, "request"))?;
    resource_id(&request.project_id, "project_id")?;
    if !request.repository_connection_id.is_empty() {
        resource_id(
            &request.repository_connection_id,
            "repository_connection_id",
        )?;
    }
    if !request.connection_id.is_empty() {
        resource_id(&request.connection_id, "connection_id")?;
    }
    git_revision(&request.repository_revision, "repository_revision")?;
    resource_id(&request.runtime_profile_id, "runtime_profile_id")?;
    let agent = request
        .agent
        .as_ref()
        .ok_or_else(|| ValidationError::new(v1::ErrorReason::InvalidInput, "agent"))?;
    safe_text(&agent.provider, 1, 128, "agent.provider")?;
    safe_text(&agent.agent, 1, 128, "agent.agent")?;
    safe_text(&agent.model, 1, 128, "agent.model")?;
    safe_text(&request.request, 1, 16_384, "request.request")?;
    if request.acceptance_criteria.is_empty()
        || request.acceptance_criteria.len() > 20
        || request
            .acceptance_criteria
            .iter()
            .any(|item| item.is_empty() || item.len() > 2_048)
        || request
            .acceptance_criteria
            .iter()
            .map(String::len)
            .sum::<usize>()
            > 16_384
    {
        return Err(ValidationError::new(
            v1::ErrorReason::BoundExceeded,
            "acceptance_criteria",
        ));
    }
    if request
        .coding_turns
        .is_some_and(|turns| !(1..=128).contains(&turns))
    {
        return Err(ValidationError::new(
            v1::ErrorReason::BoundExceeded,
            "coding_turns",
        ));
    }
    constraints(request.change_constraints.as_ref())?;
    if let Some(deadline) = request.deadline.as_ref()
        && (!(1..=86_400).contains(&deadline.seconds)
            || !(0..1_000_000_000).contains(&deadline.nanos)
            || (deadline.seconds == 86_400 && deadline.nanos != 0))
    {
        return Err(ValidationError::new(
            v1::ErrorReason::BoundExceeded,
            "deadline",
        ));
    }
    Ok(())
}

/// Validates a publication-only retry without admitting a new coding attempt.
///
/// # Errors
/// Returns the first invalid identity.
pub fn retry_task_publication(
    value: &v1::RetryTaskPublicationRequest,
) -> Result<(), ValidationError> {
    idempotency_key(&value.idempotency_key)?;
    resource_id(&value.development_task_id, "development_task_id")
}

fn provider(value: Option<&v1::RepositoryProvider>) -> Result<(), ValidationError> {
    match value.and_then(|item| item.provider.as_ref()) {
        Some(v1::repository_provider::Provider::Github(repository)) => {
            if repository.repository_id.is_empty()
                || !repository
                    .repository_id
                    .bytes()
                    .all(|byte| byte.is_ascii_digit())
                || repository.installation_id == 0
            {
                return Err(ValidationError::new(
                    v1::ErrorReason::InvalidIdentifier,
                    "provider.github",
                ));
            }
            provider_name(&repository.owner, "provider.github.owner")?;
            provider_name(&repository.name, "provider.github.name")?;
            if repository.html_url.len() > 2_048
                || !repository.html_url.starts_with("https://github.com/")
            {
                return Err(ValidationError::new(
                    v1::ErrorReason::InvalidInput,
                    "provider.github.html_url",
                ));
            }
            Ok(())
        }
        None => Err(ValidationError::new(
            v1::ErrorReason::UnknownEnumValue,
            "provider",
        )),
    }
}

fn provider_name(value: &str, field: &'static str) -> Result<(), ValidationError> {
    if !(1..=100).contains(&value.len())
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(ValidationError::new(
            v1::ErrorReason::InvalidIdentifier,
            field,
        ));
    }
    Ok(())
}

fn provider_reference(value: &str, field: &'static str) -> Result<(), ValidationError> {
    if !(1..=255).contains(&value.len())
        || !value.is_ascii()
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b' ')
    {
        return Err(ValidationError::new(
            v1::ErrorReason::InvalidIdentifier,
            field,
        ));
    }
    Ok(())
}

fn constraints(value: Option<&v1::ChangeConstraints>) -> Result<(), ValidationError> {
    let Some(value) = value else {
        return Err(ValidationError::new(
            v1::ErrorReason::InvalidInput,
            "change_constraints",
        ));
    };
    if let Some(scope) = &value.environment_change {
        if scope.dependency_paths.is_empty() {
            return Err(ValidationError::new(
                v1::ErrorReason::InvalidInput,
                "change_constraints.environment_change.dependency_paths",
            ));
        }
        constraint_paths(
            &scope.dependency_paths,
            "change_constraints.environment_change.dependency_paths",
        )?;
    }
    constraint_paths(&value.protected_paths, "change_constraints.protected_paths")
}

fn constraint_paths(paths: &[String], field: &'static str) -> Result<(), ValidationError> {
    if paths.len() > 100
        || paths.iter().map(String::len).sum::<usize>() > 16_384
        || paths.iter().any(|path| {
            path.is_empty()
                || path.len() > 512
                || path.starts_with('/')
                || path.contains('\\')
                || path.chars().any(char::is_control)
                || path
                    .split('/')
                    .any(|component| component.is_empty() || matches!(component, "." | ".."))
        })
    {
        return Err(ValidationError::new(v1::ErrorReason::BoundExceeded, field));
    }
    Ok(())
}

fn safe_text(
    value: &str,
    minimum: usize,
    maximum: usize,
    field: &'static str,
) -> Result<(), ValidationError> {
    if !(minimum..=maximum).contains(&value.len()) || value.contains('\0') {
        return Err(ValidationError::new(v1::ErrorReason::BoundExceeded, field));
    }
    Ok(())
}

fn oci_repository(value: &str) -> Result<(), ValidationError> {
    if !valid_oci_repository(value) {
        return Err(ValidationError::new(
            v1::ErrorReason::InvalidOciReference,
            "oci_repository",
        ));
    }
    Ok(())
}

fn oci_reference(value: &str, field: &'static str) -> Result<(), ValidationError> {
    if !(1..=512).contains(&value.len()) || !value.is_ascii() {
        return Err(ValidationError::new(
            v1::ErrorReason::InvalidOciReference,
            field,
        ));
    }
    let valid = if let Some((repository, digest)) = value.split_once('@') {
        !digest.contains('@')
            && valid_oci_repository(repository)
            && Sha256Digest::parse(digest).is_ok()
    } else if let Some(slash) = value.rfind('/') {
        let final_component = &value[slash + 1..];
        if let Some((name, tag)) = final_component.split_once(':') {
            let repository = format!("{}/{name}", &value[..slash]);
            !tag.contains(':') && valid_oci_repository(&repository) && valid_oci_tag(tag)
        } else {
            valid_oci_repository(value)
        }
    } else {
        false
    };
    if !valid {
        return Err(ValidationError::new(
            v1::ErrorReason::InvalidOciReference,
            field,
        ));
    }
    Ok(())
}

fn valid_oci_repository(value: &str) -> bool {
    value.contains('/') && ImageRepository::parse(value).is_ok()
}

fn valid_oci_tag(value: &str) -> bool {
    ImageTag::parse(value).is_ok()
}

fn oci_component(value: &str) -> bool {
    (1..=32).contains(&value.len())
        && value.is_ascii()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
}

fn lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

mod projects;
pub use projects::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reviewed_environment_scope_rejects_empty_and_unsafe_transport_paths()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut request = task_request();
        let constraints = request
            .request
            .as_mut()
            .ok_or("request missing")?
            .change_constraints
            .as_mut()
            .ok_or("constraints missing")?;
        constraints.environment_change = Some(v1::EnvironmentChangeAuthorization {
            dependency_paths: vec!["Cargo.toml".into(), "Cargo.lock".into()],
        });
        assert!(create_development_task(&request).is_ok());
        for paths in [
            vec![],
            vec!["../Cargo.lock"],
            vec!["/tmp/Cargo.lock"],
            vec!["Cargo\n.lock"],
        ] {
            request
                .request
                .as_mut()
                .ok_or("request missing")?
                .change_constraints
                .as_mut()
                .ok_or("constraints missing")?
                .environment_change = Some(v1::EnvironmentChangeAuthorization {
                dependency_paths: paths.into_iter().map(str::to_owned).collect(),
            });
            assert!(create_development_task(&request).is_err());
        }
        Ok(())
    }

    fn github_provider() -> v1::RepositoryProvider {
        v1::RepositoryProvider {
            provider: Some(v1::repository_provider::Provider::Github(
                v1::GithubRepository {
                    repository_id: "123".to_owned(),
                    owner: "ayni-oss".to_owned(),
                    name: "orchestrator".to_owned(),
                    installation_id: 42,
                    html_url: "https://github.com/ayni-oss/orchestrator".to_owned(),
                },
            )),
        }
    }

    fn task_request() -> v1::CreateDevelopmentTaskRequest {
        v1::CreateDevelopmentTaskRequest {
            idempotency_key: "task:1".to_owned(),
            request: Some(v1::DevelopmentTaskRequest {
                connection_id: String::new(),
                coding_turns: Some(12),
                project_id: "project:test".into(),
                repository_connection_id: "repo:1".to_owned(),
                repository_revision: "a".repeat(40),
                runtime_profile_id: "profile:1".to_owned(),
                agent: Some(v1::AgentSelection {
                    provider: "openai".to_owned(),
                    agent: "coding-agent".to_owned(),
                    model: "gpt-5".to_owned(),
                }),
                request: "Implement the bounded change".to_owned(),
                acceptance_criteria: vec!["The contract compiles".to_owned()],
                change_constraints: Some(v1::ChangeConstraints {
                    protected_paths: vec!["docs/history".to_owned()],
                    allow_test_edits: true,
                    environment_change: None,
                }),
                deadline: Some(prost_types::Duration {
                    seconds: 300,
                    nanos: 0,
                }),
            }),
        }
    }

    #[test]
    fn task_iteration_budget_is_bounded_and_optional() {
        for turns in [0, 129, u32::MAX] {
            let mut request = task_request();
            let Some(input) = request.request.as_mut() else {
                panic!("fixture request missing");
            };
            input.coding_turns = Some(turns);
            let Err(error) = create_development_task(&request) else {
                panic!("invalid budget accepted");
            };
            assert_eq!(error.field, "coding_turns");
        }
        for turns in [None, Some(1), Some(24), Some(128)] {
            let mut request = task_request();
            let Some(input) = request.request.as_mut() else {
                panic!("fixture request missing");
            };
            input.coding_turns = turns;
            assert!(create_development_task(&request).is_ok());
        }
    }

    #[test]
    fn identity_formats_are_distinct_and_strict() {
        assert!(git_revision(&"a".repeat(40), "revision").is_ok());
        assert!(git_revision(&"a".repeat(64), "revision").is_err());
        assert!(lock_fingerprint(&"b".repeat(64)).is_ok());
        assert!(lock_fingerprint(&format!("sha256:{}", "b".repeat(64))).is_err());
        assert!(oci_digest(&format!("sha256:{}", "c".repeat(64)), "digest").is_ok());
        assert!(oci_digest(&"c".repeat(64), "digest").is_err());
        assert!(oci_repository("ghcr.io/ayni-oss/runtime").is_ok());
        assert!(oci_repository("registry.example:5000/team/runtime").is_ok());
        assert!(oci_repository("ghcr.io/ayni-oss/runtime:latest").is_err());
        assert!(oci_reference("registry.example/runtime:stable", "image_reference").is_ok());
        assert!(
            oci_reference(
                &format!("registry.example/runtime@sha256:{}", "d".repeat(64)),
                "image_reference"
            )
            .is_ok()
        );
        assert!(
            oci_reference(
                "registry.example/runtime@sha256:deadbeef",
                "image_reference"
            )
            .is_err()
        );
        let maximum_reference = format!(
            "registry.example/{}/{}@sha256:{}",
            "a".repeat(211),
            "b".repeat(211),
            "d".repeat(64)
        );
        assert_eq!(maximum_reference.len(), 512);
        assert!(oci_reference(&maximum_reference, "image_reference").is_ok());
        let oversized_reference = format!(
            "registry.example/{}/{}@sha256:{}",
            "a".repeat(211),
            "b".repeat(212),
            "d".repeat(64)
        );
        assert_eq!(oversized_reference.len(), 513);
        assert!(oci_reference(&oversized_reference, "image_reference").is_err());
        assert!(idempotency_key("create:profile.1").is_ok());
        assert!(idempotency_key("contains whitespace").is_err());
        assert!(resource_id("profile:1", "profile_id").is_ok());
        assert!(resource_id("------------------------------------", "profile_id").is_err());
        assert!(enum_filter(Some(1), "state", &[1, 2]).is_ok());
        let Err(error) = enum_filter(Some(99), "state", &[1, 2]) else {
            panic!("unknown request enums must fail");
        };
        assert_eq!(error.reason, v1::ErrorReason::UnknownEnumValue);
    }

    #[test]
    fn pagination_and_task_bounds_fail_closed() {
        assert!(
            page(Some(&v1::PageRequest {
                page_size: 100,
                page_token: "x".repeat(512),
            }))
            .is_ok()
        );
        let Err(error) = page(Some(&v1::PageRequest {
            page_size: 101,
            page_token: String::new(),
        })) else {
            panic!("oversized page must fail");
        };
        assert_eq!(error.reason, v1::ErrorReason::BoundExceeded);

        let mut request = task_request();
        assert!(create_development_task(&request).is_ok());
        let Some(input) = request.request.as_mut() else {
            panic!("fixture request is absent");
        };
        input.request = "x".repeat(16_385);
        let Err(error) = create_development_task(&request) else {
            panic!("oversized task request must fail");
        };
        assert_eq!(error.field, "request.request");

        let mut request = task_request();
        let Some(input) = request.request.as_mut() else {
            panic!("fixture request is absent");
        };
        input.deadline = Some(prost_types::Duration {
            seconds: 86_400,
            nanos: 1,
        });
        let Err(error) = create_development_task(&request) else {
            panic!("deadline beyond 24 hours must fail");
        };
        assert_eq!(error.field, "deadline");
    }

    #[test]
    fn closed_profile_variants_validate_their_own_fields() {
        let ayni = v1::CreateRuntimeProfileRequest {
            idempotency_key: "profile:ayni".to_owned(),
            repository_connection_id: "repo:1".to_owned(),
            repository_snapshot_id: "snapshot:1".to_owned(),
            configuration: Some(v1::RuntimeProfileConfiguration {
                configuration: Some(
                    v1::runtime_profile_configuration::Configuration::AyniManaged(
                        v1::AyniManagedProfileRevision {
                            runtime_source_digest: None,
                            lock_fingerprint: "d".repeat(64),
                            ayni_version: "0.13.0".to_owned(),
                            oci_repository: "ghcr.io/ayni-oss/orchestrator".to_owned(),
                        },
                    ),
                ),
            }),
        };
        assert!(create_runtime_profile(&ayni).is_ok());

        let custom = v1::CreateRuntimeProfileRequest {
            idempotency_key: "profile:custom".to_owned(),
            repository_connection_id: "repo:1".to_owned(),
            repository_snapshot_id: "snapshot:1".to_owned(),
            configuration: Some(v1::RuntimeProfileConfiguration {
                configuration: Some(
                    v1::runtime_profile_configuration::Configuration::CustomRuntime(
                        v1::CustomRuntimeProfileRevision {
                            image_reference: "registry.example/runtime:stable".to_owned(),
                        },
                    ),
                ),
            }),
        };
        assert!(create_runtime_profile(&custom).is_ok());

        let absent = v1::CreateRuntimeProfileRequest {
            configuration: Some(v1::RuntimeProfileConfiguration {
                configuration: None,
            }),
            ..custom
        };
        let Err(error) = create_runtime_profile(&absent) else {
            panic!("absent profile kind must fail");
        };
        assert_eq!(error.reason, v1::ErrorReason::UnknownEnumValue);
    }

    #[test]
    fn repository_and_publication_inputs_are_bounded() {
        let repository = v1::CreateRepositoryConnectionRequest {
            use_ayni: None,
            idempotency_key: "repo:1".to_owned(),
            provider: Some(github_provider()),
            display_name: "Ayni Orchestrator".to_owned(),
            default_branch: "main".to_owned(),
            registry_connection_id: String::new(),
            image_namespace: String::new(),
        };
        assert!(create_repository_connection(&repository).is_ok());

        let publication = v1::RetryTaskPublicationRequest {
            idempotency_key: "publish:1".to_owned(),
            development_task_id: "task:1".to_owned(),
        };
        assert!(retry_task_publication(&publication).is_ok());
    }
}
