//! Workflow-owned evidence contract. Candidate declarations never establish authority.

use ayni_orchestrator_profile_ayni::managed_runtime::{RuntimeExpectation, verified_result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const VERSION: &str = "assessment-v1";
pub const CHECK_NAME: &str = "Ayni Managed assessment";
pub const MAX_BUNDLE_BYTES: usize = 10 * 1024 * 1024;
pub const MAX_MANIFEST_BYTES: usize = 16 * 1024;

/// Immutable run identity, obtained independently from GitHub by the importer.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RunIdentity {
    pub repository_id: String,
    pub pull_request: u64,
    pub head: String,
    pub base: String,
    pub profile_revision: String,
    pub workflow_id: u64,
    pub run_id: u64,
    pub attempt: u64,
    pub workflow_sha: String,
    pub producer_digest: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Succeeded,
    QualityFailed,
    Incomplete,
    PolicyRejected,
    Unsupported,
    Stale,
    Cancelled,
    InfrastructureFailed,
}

/// Written by the isolated collector after the execution container has stopped.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: String,
    pub run: RunIdentity,
    pub runtime_digest: String,
    pub lock_fingerprint: String,
    pub contract_digest: String,
    pub source_before: String,
    pub source_after: String,
    pub ayni_version: String,
    pub platform: String,
    pub exit_code: i32,
    pub outcome: Outcome,
    pub artifacts: BTreeMap<String, String>,
}

/// Provider observations and operator policy, never deserialized from evidence.
pub struct Admission<'a> {
    pub run: &'a RunIdentity,
    pub runtime_digest: &'a str,
    pub lock_fingerprint: &'a str,
    pub contract_digest: &'a str,
    pub current_head: &'a str,
    pub platform: &'a str,
    pub ayni_version: &'a str,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Summary {
    pub completed: u32,
    pub skipped: u32,
    pub warnings: u32,
    pub failures: u32,
}

/// Validates a bundle only after the caller has verified its trusted workflow
/// and collector against independent provider observations and approved policy.
///
/// # Errors
/// Rejects malformed, conflicting, stale, incomplete or unbound evidence.
pub fn validate(
    manifest_bytes: &[u8],
    artifacts: &BTreeMap<String, Vec<u8>>,
    admission: &Admission<'_>,
) -> Result<(Manifest, Summary), &'static str> {
    if manifest_bytes.len() > MAX_MANIFEST_BYTES
        || artifacts.len() > 2
        || artifacts.values().map(Vec::len).sum::<usize>() > MAX_BUNDLE_BYTES
    {
        return Err("assessment_evidence_oversized");
    }
    let manifest: Manifest =
        serde_json::from_slice(manifest_bytes).map_err(|_| "assessment_manifest_invalid")?;
    if manifest.version != VERSION
        || &manifest.run != admission.run
        || manifest.runtime_digest != admission.runtime_digest
        || manifest.lock_fingerprint != admission.lock_fingerprint
        || manifest.contract_digest != admission.contract_digest
        || manifest.platform != admission.platform
        || manifest.ayni_version != admission.ayni_version
        || !valid_run(&manifest.run)
        || !digest(&manifest.runtime_digest)
        || !digest(&manifest.lock_fingerprint)
        || !digest(&manifest.contract_digest)
    {
        return Err("assessment_identity_mismatch");
    }
    if manifest.artifacts.len() != artifacts.len()
        || artifacts.iter().any(|(name, bytes)| {
            !matches!(name.as_str(), "signals.json" | "runtime.json")
                || manifest.artifacts.get(name) != Some(&sha256(bytes))
        })
    {
        return Err("assessment_artifact_mismatch");
    }
    let summary = if matches!(
        manifest.outcome,
        Outcome::Succeeded | Outcome::QualityFailed | Outcome::Incomplete
    ) {
        validate_signals(&manifest, artifacts)?
    } else {
        Summary::default()
    };
    if matches!(
        manifest.outcome,
        Outcome::Succeeded | Outcome::QualityFailed
    ) && (!digest(&manifest.source_before) || manifest.source_before != manifest.source_after)
    {
        return Err("assessment_source_changed");
    }
    // Historical evidence can be retained, but never imported as current success.
    let mut manifest = manifest;
    if manifest.run.head != admission.current_head {
        manifest.outcome = Outcome::Stale;
    }
    Ok((manifest, summary))
}

fn validate_signals(
    manifest: &Manifest,
    artifacts: &BTreeMap<String, Vec<u8>>,
) -> Result<Summary, &'static str> {
    let signals = artifacts
        .get("signals.json")
        .ok_or("assessment_signals_missing")?;
    let metadata = artifacts
        .get("runtime.json")
        .ok_or("assessment_runtime_missing")?;
    verified_result(
        metadata,
        signals,
        &RuntimeExpectation {
            lock_fingerprint: manifest.lock_fingerprint.clone(),
            platform: manifest.platform.clone(),
            ayni_version: manifest.ayni_version.clone(),
        },
    )
    .map_err(|_| "assessment_runtime_unverified")?;
    let value: Value = serde_json::from_slice(signals).map_err(|_| "assessment_signals_invalid")?;
    if value["contract_digest"] != manifest.contract_digest
        || value["invocation"]["command"] != "check"
        || value["completion"]["scope"] != "repository"
        || invalid_tool_versions(&value["tool_versions"])
    {
        return Err("assessment_accounting_invalid");
    }
    let (completed, skipped) = validate_completion(&value)?;
    let rows = value["rows"].as_array().ok_or("assessment_rows_invalid")?;
    validate_target_rows(rows, completed, skipped)?;
    let aggregate = &value["aggregate"];
    let failing = count(aggregate, "failing_rows")?;
    if rows.len() > 4096
        || count(aggregate, "total_rows")? as usize != rows.len()
        || rows.iter().any(|row| row["pass"].as_bool().is_none())
        || rows.iter().filter(|row| row["pass"] == false).count() != failing as usize
        || count(aggregate, "passing_rows")? as usize + failing as usize != rows.len()
        || (completed > 0 && rows.is_empty())
    {
        return Err("assessment_rows_invalid");
    }
    validate_outcome(manifest, skipped, failing)?;
    Ok(Summary {
        completed,
        skipped,
        warnings: count(aggregate, "warning_offenders")?,
        failures: count(aggregate, "failing_offenders")?,
    })
}

fn validate_outcome(manifest: &Manifest, skipped: u32, failing: u32) -> Result<(), &'static str> {
    let outcome = if skipped > 0 {
        Outcome::Incomplete
    } else if failing > 0 {
        Outcome::QualityFailed
    } else {
        Outcome::Succeeded
    };
    if manifest.outcome != outcome
        || (outcome == Outcome::Succeeded && manifest.exit_code != 0)
        || (outcome == Outcome::QualityFailed && manifest.exit_code != 1)
        || (outcome == Outcome::Incomplete && manifest.exit_code != 4)
    {
        return Err("assessment_outcome_mismatch");
    }
    Ok(())
}

fn invalid_tool_versions(value: &Value) -> bool {
    value.as_array().is_none_or(|tools| {
        tools.is_empty()
            || tools.len() > 256
            || tools.iter().any(|tool| {
                ["tool", "version"].iter().any(|field| {
                    tool[field].as_str().is_none_or(|v| {
                        v.is_empty() || v.len() > 256 || v.chars().any(char::is_control)
                    })
                })
            })
    })
}

fn validate_completion(value: &Value) -> Result<(u32, u32), &'static str> {
    let completion = &value["completion"];
    let expected = count(completion, "expected_targets")?;
    let detected = count(completion, "detected_targets")?;
    let completed = count(completion, "completed_targets")?;
    let skipped = count(completion, "skipped_targets")?;
    let issues = completion["issues"]
        .as_array()
        .ok_or("assessment_accounting_invalid")?;
    if expected == 0
        || detected > expected
        || completed > detected
        || skipped != expected - completed
        || issues.len() != skipped as usize
        || issues
            .iter()
            .filter(|issue| issue["stage"] == "detection")
            .count()
            != (expected - detected) as usize
        || (skipped == 0 && completion["state"] != "complete")
        || (skipped > 0 && completion["state"] != "incomplete")
    {
        return Err("assessment_accounting_invalid");
    }
    Ok((completed, skipped))
}

fn count(value: &Value, field: &str) -> Result<u32, &'static str> {
    value[field]
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .ok_or("assessment_count_invalid")
}

fn validate_target_rows(rows: &[Value], completed: u32, skipped: u32) -> Result<(), &'static str> {
    use std::collections::BTreeSet;
    let mut targets = BTreeMap::<(&str, &str), BTreeSet<&str>>::new();
    for row in rows {
        let language = row["language"]
            .as_str()
            .ok_or("assessment_target_invalid")?;
        let kind = row["kind"].as_str().ok_or("assessment_target_invalid")?;
        let root = row["scope"]["path"]
            .as_str()
            .unwrap_or(".")
            .trim_end_matches('/');
        if language.len() > 32
            || root.len() > 1024
            || !matches!(
                kind,
                "test" | "coverage" | "size" | "complexity" | "deps" | "mutation"
            )
            || !targets
                .entry((language, if root.is_empty() { "." } else { root }))
                .or_default()
                .insert(kind)
        {
            return Err("assessment_target_invalid");
        }
    }
    if targets.len() < completed as usize || (skipped == 0 && targets.len() != completed as usize) {
        return Err("assessment_target_invalid");
    }
    if skipped == 0 {
        let mut kinds = BTreeMap::new();
        for ((language, _), set) in targets {
            if let Some(previous) = kinds.insert(language, set.clone())
                && previous != set
            {
                return Err("assessment_target_invalid");
            }
        }
    }
    Ok(())
}

pub use ayni_orchestrator_domain::digest::{digest, hex, sha256};

fn valid_run(run: &RunIdentity) -> bool {
    !run.repository_id.is_empty()
        && run.repository_id.len() <= 20
        && run.repository_id.bytes().all(|b| b.is_ascii_digit())
        && run.pull_request > 0
        && run.workflow_id > 0
        && run.run_id > 0
        && run.attempt > 0
        && hex(&run.head, 40)
        && hex(&run.base, 40)
        && hex(&run.workflow_sha, 40)
        && digest(&run.producer_digest)
        && !run.profile_revision.is_empty()
        && run.profile_revision.len() <= 128
}

#[cfg(test)]
mod tests;
