use super::*;
use std::collections::BTreeSet;
pub(super) fn sha(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|v| {
        v.len() == 64
            && v.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
pub(super) fn git_sha(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(super) fn image(value: &str, destination: Option<&str>) -> bool {
    let Some((repo, hash)) = value.split_once('@') else {
        return false;
    };
    sha(hash)
        && repo.starts_with("ghcr.io/")
        && repo.len() <= 512
        && repo.split('/').all(|p| {
            !p.is_empty()
                && p != "."
                && p != ".."
                && p.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
        })
        && destination.is_none_or(|expected| repo == expected)
}
pub(super) fn start(input: &Start<'_>) -> Result<(), StoreError> {
    validate_key(input.key)?;
    for (id, name) in [
        (input.repository_id, "repository_id"),
        (input.inspection_id, "inspection_id"),
        (input.registry_connection_id, "registry_connection_id"),
        (input.signer_id, "signer_id"),
    ] {
        validate_id(id, name)?;
    }
    if !matches!(input.platform, "linux/amd64" | "linux/arm64")
        || !(1..=86400).contains(&input.max_seconds)
        || input.max_builds > 10
        || input.allow_build != (input.max_builds > 0)
        || input.allow_replace && !input.allow_build
    {
        return Err(StoreError::InvalidInput("runtime allowance"));
    }
    Ok(())
}
pub(super) fn frozen(
    input: &FreezeInputs,
    start: &Start<'_>,
) -> Result<RuntimeProfile, StoreError> {
    if !git_sha(&input.source_tree)
        || input.policy.len() > 524_288
        || input.lock.len() > 524_288
        || input.sources.len() > 256
        || input.profile_json.len() > 16384
    {
        return Err(StoreError::InvalidInput("frozen runtime inputs"));
    }
    let profile: RuntimeProfile = serde_json::from_str(&input.profile_json)
        .map_err(|_| StoreError::InvalidInput("runtime profile"))?;
    qualified_profile(&profile, start)?;
    let mut total = 0usize;
    let mut names = BTreeSet::new();
    for file in &input.sources {
        if file.path.is_empty()
            || file.path.len() > 4096
            || file.path.contains('\\')
            || file.path.chars().any(char::is_control)
            || file
                .path
                .split('/')
                .any(|p| p.is_empty() || p == "." || p == ".." || p.contains(':'))
            || !names.insert(&file.path)
            || file.content.len() > 524_288
            || digest(&file.content) != file.digest
        {
            return Err(StoreError::InvalidInput("locked source input"));
        }
        if file.path == ".ayni.toml" && file.content != input.policy.as_bytes()
            || file.path == ".ayni.lock" && file.content != input.lock.as_bytes()
        {
            return Err(StoreError::InvalidInput("locked header mismatch"));
        }
        total = total.saturating_add(file.content.len());
    }
    if !names.contains(&String::from(".ayni.toml")) {
        total = total.saturating_add(input.policy.len());
    }
    if !names.contains(&String::from(".ayni.lock")) {
        total = total.saturating_add(input.lock.len());
    }
    if total > 8 * 1024 * 1024 {
        return Err(StoreError::InvalidInput("frozen input bounds"));
    }
    Ok(profile)
}
fn safe_reason(value: &str) -> bool {
    matches!(
        value,
        "" | "none"
            | "setup_failure"
            | "builder_unavailable"
            | "invalid_input"
            | "source_unavailable"
            | "source_access_denied"
            | "source_rate_limited"
            | "source_identity_mismatch"
            | "source_path_unsupported"
            | "source_bounds_exceeded"
            | "tool_unavailable"
            | "execution_failed"
            | "execution_deadline"
            | "execution_output_exceeded"
            | "artifact_invalid"
            | "artifact_conflict"
            | "artifact_bounds_exceeded"
            | "platform_mismatch"
            | "registry_access_denied"
            | "registry_unavailable"
            | "publication_uncertain"
            | "certification_failed"
            | "compatibility_failed"
            | "cleanup_pending"
            | "stage_failed"
            | "workload_interrupted"
            | "deadline_exceeded"
            | "selection_superseded"
            | "build_allowance_exhausted"
            | "image_missing"
            | "replacement_required"
            | "cancelled"
    )
}
pub(super) fn report(report: &StageReport) -> Result<(), StoreError> {
    if !ayni_orchestrator_events::runtime_preparation::STAGES
        .contains(&report.binding.stage.as_str())
        || !matches!(
            report.outcome.as_str(),
            "completed"
                | "missing"
                | "invalid"
                | "failed"
                | "unavailable"
                | "publication_uncertain"
                | "cleanup_pending"
        )
        || !safe_reason(&report.safe_reason)
    {
        return Err(StoreError::InvalidInput("runtime stage outcome"));
    }
    let e = &report.evidence;
    for value in [
        &e.source_digest,
        &e.managed_check_digest,
        &e.unsigned_manifest_digest,
        &e.certification_layer_digest,
        &e.artifact_digest,
        &e.protected_content_root,
        &e.certificate_digest,
        &e.expected_manifest_digest,
        &e.image_digest,
    ] {
        if !value.is_empty() && !sha(value) {
            return Err(StoreError::InvalidInput("runtime evidence digest"));
        }
    }
    if !e.candidate_image.is_empty() && !image(&e.candidate_image, None)
        || e.artifact_reference.len() > 512
        || e.artifact_reference.chars().any(char::is_control)
        || json(e)?.len() > 16384
    {
        return Err(StoreError::InvalidInput("runtime stage evidence"));
    }
    Ok(())
}

fn qualified_profile(profile: &RuntimeProfile, start: &Start<'_>) -> Result<(), StoreError> {
    if !ayni_orchestrator_events::runtime_preparation::admitted_profile_identity(
        &profile.profile_id,
        profile.qualified,
    ) || profile.platform != start.platform
        || profile.max_seconds < start.max_seconds
        || profile.max_builds < start.max_builds
        || profile.max_cpu_millis == 0
        || profile.max_memory_mib == 0
        || profile.max_storage_mib == 0
        || !sha(&profile.trusted_cli_sha256)
        || [
            &profile.worker_image,
            &profile.executor_image,
            &profile.verifier_image,
        ]
        .iter()
        .any(|v| !ayni_orchestrator_events::runtime_preparation::valid_tool_image_reference(v))
    {
        return Err(StoreError::InvalidInput("qualified runtime profile"));
    }
    Ok(())
}
