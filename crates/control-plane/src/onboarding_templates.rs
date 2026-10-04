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

pub(crate) const TEMPLATE_VERSION: &str = "ayni-managed-v3";
pub(crate) const ENVIRONMENT_WORKFLOW_PATH: &str = ".github/workflows/ayni-environment.yml";
pub(crate) const TRIGGER_WORKFLOW_PATH: &str = ".github/workflows/ayni-assessment-request.yml";
pub(crate) const ASSESSMENT_WORKFLOW_PATH: &str = ".github/workflows/ayni-managed-assessment.yml";

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
        files: BTreeMap::from([
            (
                ENVIRONMENT_WORKFLOW_PATH,
                pinned_workflow(include_str!("../templates/environment.yml")).replace(
                    "@@DEFAULT_BRANCH@@",
                    &serde_json::to_string(default_branch)
                        .map_err(|_| TemplateError::InvalidDefaultBranch(default_branch.into()))?,
                ),
            ),
            (ASSESSMENT_WORKFLOW_PATH, assessment_workflow()),
            (TRIGGER_WORKFLOW_PATH, trigger_workflow()),
        ]),
    })
}

fn assessment_workflow() -> String {
    pinned_workflow(include_str!("../templates/assessment.yml"))
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

fn trigger_workflow() -> String {
    include_str!("../templates/assessment-trigger.yml").to_owned()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn renders_deterministic_pinned_workflows() {
        let first = render(TEMPLATE_VERSION, "main").unwrap();
        assert_eq!(first, render(TEMPLATE_VERSION, "main").unwrap());
        assert_eq!(first.files.len(), 3);
        for workflow in first.files.values() {
            assert!(workflow.contains("ayni-orchestrator-template: ayni-managed-v3"));
            assert!(workflow.contains("concurrency:"));
            assert!(!workflow.contains("@main"));
            assert!(!workflow.contains("--host"));
            assert!(!workflow.contains("env shell"));
            assert!(!workflow.contains("env run"));
        }
    }

    #[test]
    fn assessment_workflow_never_receives_build_or_registry_credentials() {
        let files = render(TEMPLATE_VERSION, "main").unwrap().files;
        let assessment = &files[ASSESSMENT_WORKFLOW_PATH];
        assert!(assessment.contains("workflow_run:"));
        assert!(!assessment.contains("@@AYNI_"));
        assert!(assessment.contains(ayni_orchestrator_profile_ayni::ayni_release::CLI_VERSION));
        assert!(
            assessment
                .contains(ayni_orchestrator_profile_ayni::ayni_release::LINUX_AMD64_ARCHIVE_SHA256)
        );
        assert!(assessment.contains("contents: read"));
        assert!(!assessment.contains("packages: write"));
        assert!(!assessment.contains("AYNI_ENV_CERTIFICATE_SIGNING_KEY"));
        assert!(assessment.contains("packages: read"));
        assert!(!assessment.contains("docker push"));
    }

    #[test]
    fn rejects_unknown_template_versions() {
        assert!(render("unreviewed", "main").is_err());
        assert!(render(TEMPLATE_VERSION, "bad branch").is_err());
    }
}
