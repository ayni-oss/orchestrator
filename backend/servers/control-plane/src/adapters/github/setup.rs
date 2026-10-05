//! Credential-free setup-PR intent for the GitHub onboarding adapter.

use crate::profiles::ayni::onboarding_templates::SetupWorkflowFiles;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// The deterministic GitHub mutation requested by an Ayni Managed profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupPullRequest {
    pub(crate) branch: String,
    pub(crate) base_branch: String,
    pub(crate) title: &'static str,
    pub(crate) body: &'static str,
    pub(crate) files: SetupWorkflowFiles,
}

/// A repository identity cannot safely form an owned setup branch.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum SetupRequestError {
    #[error("setup branch identity is empty or exceeds its bound")]
    InvalidProfileIdentity,
    #[error("setup PR cannot be created without the managed assessment workflow")]
    MissingWorkflow,
}

/// Builds a stable request that recovery can look up before attempting creation.
///
/// Authentication is deliberately absent from the result. An adapter obtains a
/// short-lived installation token only at call time and never retains it.
pub fn setup_pull_request(
    profile_id: &str,
    base_branch: &str,
    files: SetupWorkflowFiles,
) -> Result<SetupPullRequest, SetupRequestError> {
    if profile_id.is_empty() || profile_id.len() > 256 {
        return Err(SetupRequestError::InvalidProfileIdentity);
    }
    if files.files.len() != 3
        || !files
            .files
            .contains_key(crate::profiles::ayni::onboarding_templates::ASSESSMENT_WORKFLOW_PATH)
    {
        return Err(SetupRequestError::MissingWorkflow);
    }
    let digest = Sha256::digest(format!("ayni.setup-pr.v1\0{profile_id}\0{}", files.version));
    Ok(SetupPullRequest {
        branch: format!("setup/runtime-{digest:x}")[..48].to_owned(),
        base_branch: base_branch.to_owned(),
        title: "feat(ayni): install managed environment workflows",
        body: "This controller-owned setup PR installs the reviewed environment producer and managed assessment workflows. Review and merge it to enable managed assessment.",
        files,
    })
}

/// Observe the expected files before requesting any provider write permission.
/// A missing/different file needs setup; an observation error must stay an error.
///
/// # Errors
/// Propagates provider failures rather than treating inaccessible files as absent.
pub async fn installed_workflows_match<F, R>(
    files: &SetupWorkflowFiles,
    mut read: F,
) -> Result<bool, &'static str>
where
    F: FnMut(&'static str) -> R,
    R: std::future::Future<Output = Result<Vec<u8>, &'static str>>,
{
    let mut matches = true;
    for (path, expected) in &files.files {
        match read(path).await {
            Ok(actual) => matches &= actual == expected.as_bytes(),
            Err("github_not_found") => matches = false,
            Err(reason) => return Err(reason),
        }
    }
    Ok(matches)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::profiles::ayni::onboarding_templates::{TEMPLATE_VERSION, render};

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
        assert!(first.branch.starts_with("setup/runtime-"));
        assert_eq!(first.files.files.len(), 3);
    }
    #[tokio::test]
    async fn existing_workflows_need_no_setup_but_observation_errors_fail_closed() {
        let files = render(TEMPLATE_VERSION, "main").unwrap();
        assert_eq!(
            installed_workflows_match(&files, |path| std::future::ready(Ok(files.files[path]
                .as_bytes()
                .to_vec())))
            .await,
            Ok(true)
        );
        assert_eq!(
            installed_workflows_match(&files, |_| std::future::ready(Ok(b"different".to_vec())))
                .await,
            Ok(false)
        );
        assert_eq!(
            installed_workflows_match(&files, |_| std::future::ready(Err("github_not_found")))
                .await,
            Ok(false)
        );
        for reason in [
            "github_access_denied",
            "github_rate_limited",
            "github_unavailable",
        ] {
            assert_eq!(
                installed_workflows_match(&files, |_| std::future::ready(Err(reason))).await,
                Err(reason)
            );
        }
    }
}
