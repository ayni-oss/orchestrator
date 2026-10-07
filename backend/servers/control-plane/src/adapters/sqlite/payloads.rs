use super::{timestamp, unix_seconds, v1};

pub(super) fn safe_metadata(
    values: &[v1::VerifiedMetadata],
    max_count: usize,
    max_bytes: usize,
) -> bool {
    values.len() <= max_count
        && values
            .iter()
            .map(|value| value.key.len().saturating_add(value.value.len()))
            .sum::<usize>()
            <= max_bytes
        && values.iter().enumerate().all(|(index, value)| {
            !value.key.is_empty()
                && value.key.len() <= 64
                && value.value.len() <= 512
                && value.key.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'_' | b'-' | b'.')
                })
                && !values[..index].iter().any(|other| other.key == value.key)
        })
}

pub(super) fn valid_git_revision(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn valid_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn valid_artifact_report(artifact: &v1::RuntimeArtifact) -> bool {
    let platform_valid = artifact.platform.as_ref().is_some_and(|platform| {
        !platform.operating_system.is_empty()
            && platform.operating_system.len() <= 32
            && !platform.architecture.is_empty()
            && platform.architecture.len() <= 32
            && platform
                .variant
                .as_ref()
                .is_none_or(|value| value.len() <= 32)
    });
    artifact.status != v1::RuntimeArtifactStatus::Unspecified as i32
        && v1::RuntimeArtifactStatus::try_from(artifact.status).is_ok()
        && !artifact.oci_repository.is_empty()
        && artifact.oci_repository.len() <= 512
        && valid_digest(&artifact.resolved_digest)
        && platform_valid
        && safe_metadata(&artifact.verified_metadata, 32, 8_192)
}

pub(super) fn valid_usage_summary(summary: Option<&v1::ModelUsageSummary>) -> bool {
    summary.is_none_or(|summary| {
        summary
            .estimated_cost_currency
            .as_ref()
            .is_none_or(|value| value.len() <= 64)
            && summary
                .pricing_version
                .as_ref()
                .is_none_or(|value| value.len() <= 64)
    })
}

pub(super) fn valid_model_usage(usage: Option<&v1::AttemptModelUsage>) -> bool {
    usage.is_some_and(|usage| {
        usage.collection_status != v1::ModelUsageCollectionStatus::Unspecified as i32
            && v1::ModelUsageCollectionStatus::try_from(usage.collection_status).is_ok()
            && usage.interactions.len() <= 64
            && usage.interactions.iter().all(|interaction| {
                !interaction.call_id.is_empty()
                    && interaction.call_id.len() <= 128
                    && interaction.provider.len() <= 128
                    && interaction.model.len() <= 128
                    && interaction
                        .resolved_model
                        .as_ref()
                        .is_none_or(|value| value.len() <= 128)
                    && interaction.stage != v1::ModelUsageStage::Unspecified as i32
                    && v1::ModelUsageStage::try_from(interaction.stage).is_ok()
                    && interaction.outcome != v1::ModelInteractionOutcome::Unspecified as i32
                    && v1::ModelInteractionOutcome::try_from(interaction.outcome).is_ok()
                    && interaction.estimated_cost.as_ref().is_none_or(|cost| {
                        cost.label.len() <= 64
                            && cost.currency.len() <= 64
                            && cost.pricing_version.len() <= 64
                    })
            })
            && usage.by_stage.len() <= 2
            && usage.by_stage.iter().enumerate().all(|(index, stage)| {
                stage.stage != v1::ModelUsageStage::Unspecified as i32
                    && v1::ModelUsageStage::try_from(stage.stage).is_ok()
                    && valid_usage_summary(stage.summary.as_ref())
                    && !usage.by_stage[..index]
                        .iter()
                        .any(|other| other.stage == stage.stage)
            })
            && valid_usage_summary(usage.total.as_ref())
    })
}

pub(super) fn same_artifact_report(
    existing: &v1::RuntimeArtifact,
    incoming: &v1::RuntimeArtifact,
) -> bool {
    existing.runtime_profile_revision_id == incoming.runtime_profile_revision_id
        && existing.oci_repository == incoming.oci_repository
        && existing.discovery_tag == incoming.discovery_tag
        && existing.resolved_digest == incoming.resolved_digest
        && existing.platform == incoming.platform
        && existing.origin == incoming.origin
        && existing.status == incoming.status
        && existing.verified_metadata == incoming.verified_metadata
        && equivalent_failure(existing.failure.as_ref(), incoming.failure.as_ref())
}

pub(super) fn equivalent_failure(
    existing: Option<&v1::ResourceFailure>,
    incoming: Option<&v1::ResourceFailure>,
) -> bool {
    match (existing, incoming) {
        (None, None) => true,
        (Some(existing), Some(incoming)) => {
            existing.reason == incoming.reason
                && existing.message == incoming.message
                && existing.remediation == incoming.remediation
        }
        _ => false,
    }
}

pub(super) fn bounded_failure(
    mut failure: Option<v1::ResourceFailure>,
) -> Option<v1::ResourceFailure> {
    if let Some(value) = failure.as_mut() {
        value.message = truncate_utf8(&value.message, 512);
        value.remediation = value
            .remediation
            .take()
            .map(|text| truncate_utf8(&text, 1_024));
        if value.observed_at.is_none() {
            value.observed_at = unix_seconds().ok().map(timestamp);
        }
    }
    failure
}

pub(super) fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    let mut end = value.len().min(max_bytes);
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    value[..end].to_owned()
}
