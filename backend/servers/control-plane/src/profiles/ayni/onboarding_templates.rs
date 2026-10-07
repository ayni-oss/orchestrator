//! Versioned, deterministic GitHub Actions templates for Ayni Managed onboarding.
//!
//! These templates are data owned by the controller. They are never assembled
//! from repository input and deliberately contain neither signing values nor a
//! registry credential. The GitHub provider is responsible for committing the
//! returned files only to the deterministic setup branch.
#![allow(
    clippy::redundant_pub_crate,
    reason = "the controller exposes templates to sibling reconciliation modules, not downstream crates"
)]

use std::collections::BTreeMap;
use thiserror::Error;

pub(crate) const TEMPLATE_VERSION: &str = "ayni-managed-v4";
pub(crate) const ENVIRONMENT_WORKFLOW_PATH: &str = ".github/workflows/ayni-environment.yml";
/// Immutable files proposed by an Ayni Managed setup PR.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SetupWorkflowFiles {
    pub(crate) version: &'static str,
    pub(crate) files: BTreeMap<&'static str, String>,
}

/// A template cannot safely be used for onboarding.
#[derive(Debug, Error, Eq, PartialEq)]
pub(crate) enum TemplateError {
    #[error("workflow template version is unsupported: {0:?}")]
    UnsupportedVersion(String),
    #[error("default branch is unsafe for a workflow trigger: {0:?}")]
    InvalidDefaultBranch(String),
}

/// Renders the complete, fixed set of workflow files for one profile revision.
///
/// # Errors
///
/// Returns an error for a template version unknown to this controller.
pub(crate) fn render(
    version: &str,
    default_branch: &str,
) -> Result<SetupWorkflowFiles, TemplateError> {
    if version != TEMPLATE_VERSION {
        return Err(TemplateError::UnsupportedVersion(version.to_owned()));
    }
    if default_branch.is_empty()
        || default_branch.len() > 255
        || default_branch
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b' ')
    {
        return Err(TemplateError::InvalidDefaultBranch(
            default_branch.to_owned(),
        ));
    }
    Ok(SetupWorkflowFiles {
        version: TEMPLATE_VERSION,
        files: BTreeMap::from([(
            ENVIRONMENT_WORKFLOW_PATH,
            pinned_workflow(include_str!("../../../templates/environment.yml")).replace(
                "@@DEFAULT_BRANCH@@",
                &serde_json::to_string(default_branch)
                    .map_err(|_| TemplateError::InvalidDefaultBranch(default_branch.into()))?,
            ),
        )]),
    })
}

fn pinned_workflow(template: &str) -> String {
    template
        .replace(
            "@@AYNI_EXECUTOR_IMAGE@@",
            ayni_orchestrator_profile_ayni::ayni_release::EXECUTOR_IMAGE,
        )
        .replace(
            "@@AYNI_VERSION@@",
            ayni_orchestrator_profile_ayni::ayni_release::CLI_VERSION,
        )
        .replace(
            "@@AYNI_SHA256@@",
            ayni_orchestrator_profile_ayni::ayni_release::LINUX_AMD64_ARCHIVE_SHA256,
        )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn renders_deterministic_pinned_workflows() {
        let first = render(TEMPLATE_VERSION, "main").unwrap();
        assert_eq!(first, render(TEMPLATE_VERSION, "main").unwrap());
        assert_eq!(first.files.len(), 1);
        for workflow in first.files.values() {
            assert!(workflow.contains("ayni-orchestrator-template: ayni-managed-v4"));
            assert!(workflow.contains("concurrency:"));
            assert!(!workflow.contains("@main"));
            assert!(!workflow.contains("--host"));
            assert!(!workflow.contains("env shell"));
            assert!(!workflow.contains("env run"));
        }
    }

    #[test]
    fn onboarding_contains_the_environment_workflow() {
        let files = render(TEMPLATE_VERSION, "main").unwrap().files;
        assert!(files.contains_key(ENVIRONMENT_WORKFLOW_PATH));
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn rejects_unknown_template_versions() {
        assert!(render("unreviewed", "main").is_err());
        assert!(render(TEMPLATE_VERSION, "bad branch").is_err());
    }
}
