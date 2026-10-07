//! Shared change constraints, change bounds, and model usage contracts.

pub mod digest;
mod usage;

pub use usage::{ModelInteractionOutcome, ModelUsageReport, ModelUsageStage, ProviderTokenUsage};

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;

/// Maximum changed paths retained for one bounded development task.
pub const MAX_CHANGED_PATHS: usize = 256;
/// Maximum aggregate UTF-8 bytes retained for a task's changed paths.
pub const MAX_CHANGED_PATH_BYTES: usize = 65_536;
/// Maximum scope violations retained for one bounded task.
pub const MAX_SCOPE_VIOLATIONS: usize = MAX_CHANGED_PATHS * 2;
/// Maximum aggregate UTF-8 bytes retained for scope violations.
pub const MAX_SCOPE_VIOLATION_BYTES: usize = 524_288;

/// Returns whether scope-violation evidence is within durable storage and API bounds.
#[must_use]
pub fn scope_violations_are_bounded(violations: &[String]) -> bool {
    violations.len() <= MAX_SCOPE_VIOLATIONS
        && violations
            .iter()
            .try_fold(0_usize, |total, violation| {
                total.checked_add(violation.len())
            })
            .is_some_and(|total| total <= MAX_SCOPE_VIOLATION_BYTES)
}

/// Immutable limits on the files and test code a development task may change.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeConstraints {
    /// Repository-relative files or directory trees the task must not change.
    #[serde(default)]
    pub protected_paths: Vec<String>,
    /// Whether changes to dedicated or inline Rust tests are authorized.
    #[serde(default)]
    pub allow_test_edits: bool,
}

impl ChangeConstraints {
    /// Validates bounded, normalized repository-relative protected paths.
    ///
    /// # Errors
    ///
    /// Returns [`ChangeConstraintsError`] for an invalid, duplicate, or oversized
    /// protected-path set.
    pub fn validate(&self) -> Result<(), ChangeConstraintsError> {
        if self.protected_paths.len() > 100 {
            return Err(ChangeConstraintsError::TooManyProtectedPaths(
                self.protected_paths.len(),
            ));
        }
        if self.protected_paths.iter().map(String::len).sum::<usize>() > 16_384 {
            return Err(ChangeConstraintsError::ProtectedPathsTooLarge);
        }
        let mut unique = BTreeSet::new();
        for path in &self.protected_paths {
            if !repository_path_is_valid(path) {
                return Err(ChangeConstraintsError::InvalidProtectedPath(path.clone()));
            }
            if !unique.insert(path) {
                return Err(ChangeConstraintsError::DuplicateProtectedPath(path.clone()));
            }
        }
        Ok(())
    }

    /// Returns the configured path that protects `changed_path`, including a
    /// protected directory's descendants.
    #[must_use]
    pub fn protecting_path<'a>(&'a self, changed_path: &str) -> Option<&'a str> {
        self.protected_paths.iter().find_map(|protected| {
            (changed_path == protected
                || changed_path
                    .strip_prefix(protected.as_str())
                    .is_some_and(|suffix| suffix.starts_with('/')))
            .then_some(protected.as_str())
        })
    }
}

/// Returns whether a path is normalized, bounded, and relative to a repository root.
#[must_use]
pub fn repository_path_is_valid(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 512
        && !path.starts_with('/')
        && !path.ends_with('/')
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|component| !component.is_empty() && !matches!(component, "." | ".."))
}

/// A submitted task's change constraints are unsafe or exceed supported bounds.
#[derive(Debug, Error)]
pub enum ChangeConstraintsError {
    /// More protected paths were supplied than the bounded task model permits.
    #[error("at most 100 protected paths are allowed, got {0}")]
    TooManyProtectedPaths(usize),
    /// The combined protected-path input exceeded its bounded storage size.
    #[error("protected paths must total at most 16384 bytes")]
    ProtectedPathsTooLarge,
    /// A protected path was not normalized and repository-relative.
    #[error("protected path must be normalized and repository-relative, got {0:?}")]
    InvalidProtectedPath(String),
    /// The same protected path was submitted more than once.
    #[error("protected path was provided more than once: {0:?}")]
    DuplicateProtectedPath(String),
}

#[cfg(test)]
mod tests {
    use super::{ChangeConstraints, MAX_SCOPE_VIOLATIONS, scope_violations_are_bounded};

    #[test]
    fn validates_and_matches_frozen_protected_paths() {
        let constraints = ChangeConstraints {
            protected_paths: vec!["tests".into(), "src/generated.rs".into()],
            allow_test_edits: false,
        };
        assert!(constraints.validate().is_ok());
        assert_eq!(constraints.protecting_path("tests/api.rs"), Some("tests"));
        assert_eq!(
            constraints.protecting_path("src/generated.rs"),
            Some("src/generated.rs")
        );
        assert_eq!(constraints.protecting_path("src/generated.rs.bak"), None);
        assert_eq!(constraints.protecting_path("src/lib.rs"), None);

        for path in [
            "",
            "/tmp/file",
            "../outside",
            "src/../tests",
            "src\\main.rs",
        ] {
            assert!(
                ChangeConstraints {
                    protected_paths: vec![path.into()],
                    allow_test_edits: false,
                }
                .validate()
                .is_err(),
                "accepted unsafe path {path:?}"
            );
        }
        assert!(
            ChangeConstraints {
                protected_paths: vec!["tests".into(), "tests".into()],
                allow_test_edits: false,
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn bounds_retained_scope_violations() {
        assert!(scope_violations_are_bounded(&["violation".into()]));
        assert!(!scope_violations_are_bounded(&vec![
            "violation".into();
            MAX_SCOPE_VIOLATIONS + 1
        ]));
        assert!(!scope_violations_are_bounded(&[
            "x".repeat(super::MAX_SCOPE_VIOLATION_BYTES + 1),
        ]));
    }
}
