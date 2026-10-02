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

pub(crate) const TEMPLATE_VERSION: &str = "ayni-managed-v1";
pub(crate) const ASSESSMENT_WORKFLOW_PATH: &str = ".github/workflows/ayni-managed-assessment.yml";

const AYNI_VERSION: &str = "0.14.2";
const AYNI_ARCHIVE_SHA256: &str =
    "73579859f48d57aec892ca382a1d740990b187d0ebccc2c8646aa34d821927ac";
const CHECKOUT_ACTION: &str = "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1";

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
        files: BTreeMap::from([(ASSESSMENT_WORKFLOW_PATH, assessment_workflow())]),
    })
}

fn header() -> String {
    format!(
        "# ayni-orchestrator-template: {TEMPLATE_VERSION}\n# This file is controller-owned; edits require setup-PR review.\n"
    )
}

fn install_ayni() -> String {
    format!(
        "      - name: Install pinned Ayni\n        shell: bash\n        run: |\n          set -euo pipefail\n          archive=\"$RUNNER_TEMP/ayni.tar.gz\"\n          curl --fail --location --proto '=https' --tlsv1.2 --retry 3 --output \"$archive\" \\\n+            \"https://github.com/ayni-oss/ayni/releases/download/ayni-v{AYNI_VERSION}/ayni-ayni-v{AYNI_VERSION}-x86_64-unknown-linux-gnu.tar.gz\"\n          echo \"{AYNI_ARCHIVE_SHA256}  $archive\" | sha256sum --check\n          mkdir -p \"$RUNNER_TEMP/ayni-bin\"\n          tar --extract --gzip --file \"$archive\" --directory \"$RUNNER_TEMP/ayni-bin\"\n          echo \"$RUNNER_TEMP/ayni-bin\" >> \"$GITHUB_PATH\"\n"
    )
}

fn assessment_workflow() -> String {
    format!(
        "{}name: Ayni Managed assessment\n\non:\n  pull_request:\n    types: [opened, reopened, synchronize]\n\nconcurrency:\n  group: ayni-managed-assessment-${{{{ github.event.pull_request.number }}}}\n  cancel-in-progress: true\n\npermissions:\n  contents: read\n\njobs:\n  assess:\n    runs-on: ubuntu-24.04\n    timeout-minutes: 30\n    steps:\n      - name: Checkout exact pull-request head\n        uses: {CHECKOUT_ACTION} # v7.0.1\n        with:\n          ref: ${{{{ github.event.pull_request.head.sha }}}}\n          persist-credentials: false\n{}      - name: Check the admitted runtime\n        shell: bash\n        env:\n          AYNI_RUNTIME_IMAGE: ${{{{ vars.AYNI_RUNTIME_IMAGE }}}}\n        run: |\n          set -euo pipefail\n          test -n \"$AYNI_RUNTIME_IMAGE\"\n          case \"$AYNI_RUNTIME_IMAGE\" in *@sha256:*) ;; *) echo \"Ayni runtime must be digest-pinned\" >&2; exit 1 ;; esac\n          docker run --rm --read-only --user 10002:10001 -v \"$GITHUB_WORKSPACE:/workspace\" -w /workspace \"$AYNI_RUNTIME_IMAGE\" ayni check\n",
        header(),
        install_ayni(),
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
            assert!(workflow.contains("ayni-orchestrator-template: ayni-managed-v1"));
            assert!(workflow.contains(CHECKOUT_ACTION));
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
        assert!(assessment.contains("pull_request:"));
        assert!(assessment.contains("contents: read"));
        assert!(!assessment.contains("packages: write"));
        assert!(!assessment.contains("AYNI_ENV_CERTIFICATE_SIGNING_KEY"));
        assert!(!assessment.contains("docker login"));
        assert!(!assessment.contains("docker push"));
    }

    #[test]
    fn rejects_unknown_template_versions() {
        assert!(render("unreviewed", "main").is_err());
        assert!(render(TEMPLATE_VERSION, "bad branch").is_err());
    }
}
