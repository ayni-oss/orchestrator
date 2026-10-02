//! Credential-free setup-PR intent for the GitHub onboarding adapter.

use crate::onboarding_templates::SetupWorkflowFiles;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// The deterministic GitHub mutation requested by an Ayni Managed profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SetupPullRequest {
    pub(crate) branch: String,
    pub(crate) base_branch: String,
    pub(crate) title: &'static str,
    pub(crate) body: &'static str,
    pub(crate) files: SetupWorkflowFiles,
}

/// A repository identity cannot safely form an owned setup branch.
#[derive(Debug, Error, Eq, PartialEq)]
pub(crate) enum SetupRequestError {
    #[error("setup branch identity is empty or exceeds its bound")]
    InvalidProfileIdentity,
    #[error("setup PR cannot be created without both workflow files")]
    MissingWorkflow,
}

/// Builds a stable request that recovery can look up before attempting creation.
///
/// Authentication is deliberately absent from the result. An adapter obtains a
/// short-lived installation token only at call time and never retains it.
pub(crate) fn setup_pull_request(
    profile_id: &str,
    base_branch: &str,
    files: SetupWorkflowFiles,
) -> Result<SetupPullRequest, SetupRequestError> {
    if profile_id.is_empty() || profile_id.len() > 256 {
        return Err(SetupRequestError::InvalidProfileIdentity);
    }
    if files.files.len() != 2
        || !files
            .files
            .contains_key(crate::onboarding_templates::ASSESSMENT_WORKFLOW_PATH)
        || !files
            .files
            .contains_key(crate::onboarding_templates::ENVIRONMENT_WORKFLOW_PATH)
    {
        return Err(SetupRequestError::MissingWorkflow);
    }
    let digest = Sha256::digest(format!("ayni.setup-pr.v1\0{profile_id}\0{}", files.version));
    Ok(SetupPullRequest {
        branch: format!("ayni-orchestrator/setup-{:x}", digest)[..48].to_owned(),
        base_branch: base_branch.to_owned(),
        title: "feat(ayni): install managed environment workflows",
        body: "This controller-owned setup PR installs the reviewed Ayni Managed workflows. Review and merge it to begin environment reconciliation.",
        files,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::onboarding_templates::{TEMPLATE_VERSION, render};

    #[test]
    fn recovery_uses_one_deterministic_branch_without_default_branch_mutation() {
        let first = setup_pull_request(
            "profile:123",
            "main",
            render(TEMPLATE_VERSION, "main").unwrap(),
        )
        .unwrap();
        let second = setup_pull_request(
            "profile:123",
            "main",
            render(TEMPLATE_VERSION, "main").unwrap(),
        )
        .unwrap();
        assert_eq!(first.branch, second.branch);
        assert_ne!(first.branch, first.base_branch);
        assert!(first.branch.starts_with("ayni-orchestrator/setup-"));
        assert_eq!(first.files.files.len(), 2);
    }
}
