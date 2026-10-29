//! Static Ayni evaluation over bounded immutable Git objects. This never executes repository code.
use super::github::{Github, safe_path};
use ayni_core::{AyniPolicy, EnvironmentLock, sha256_fingerprint};
use ayni_orchestrator_workflow_postgres::store::{
    repositories::Repository,
    repository_inspections::{Finding, InspectionResult},
};
use futures_util::{StreamExt, stream};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};

const POLICY: &str = ".ayni.toml";
const LOCK: &str = ".ayni.lock";
const TOTAL_BYTES: usize = 8 * 1024 * 1024;

fn finding(code: &str, path: &str, action: &str) -> Finding {
    Finding {
        code: code.into(),
        path: path.into(),
        action: action.into(),
    }
}
fn source_finding(reason: &str, path: &str) -> Finding {
    match reason {
        "source_bounds_exceeded" => finding(
            reason,
            path,
            "Reduce locked inputs to at most 256 regular files, 512 KiB per file and 8 MiB total; retry inspection at a new commit.",
        ),
        _ => finding(
            "source_path_unsupported",
            path,
            "Use repository-relative regular files for policy, lock and all locked inputs; remove symlinks, submodules and unsafe paths.",
        ),
    }
}
fn empty_result() -> InspectionResult {
    InspectionResult {
        policy_digest: String::new(),
        lock_digest: String::new(),
        lock_fingerprint: String::new(),
        ayni_version: String::new(),
        lock_schema: String::new(),
        tools: vec![],
        trust_key_ids: vec![],
        findings: vec![
            finding(
                "adapter_discovery_pending",
                LOCK,
                "Full adapter discovery and environment compatibility must pass before runtime preparation; this inspection validates static locked inputs only.",
            ),
            finding(
                "runtime_verification_pending",
                LOCK,
                "Prepare and verify an immutable runtime before selecting this repository for execution.",
            ),
        ],
    }
}

pub(super) async fn inspect(
    client: &Github,
    repo: &Repository,
    commit: &str,
) -> Result<InspectionResult, &'static str> {
    let mut result = empty_result();
    let source = match client
        .frozen_source(repo.installation_id, repo.provider_repository_id, commit)
        .await
    {
        Ok(source) => source,
        Err(reason @ ("source_bounds_exceeded" | "source_path_unsupported")) => {
            result.findings.push(source_finding(reason, LOCK));
            return Ok(result);
        }
        Err(reason) => return Err(reason),
    };
    let mut files = BTreeMap::new();
    for path in [POLICY, LOCK] {
        record_header(
            &mut result,
            &mut files,
            path,
            client.frozen_blob(&source, path).await,
        )?;
    }
    let (mut result, lock) = inspect_headers(result, &files);
    let Some(lock) = lock else { return Ok(result) };
    let paths = match locked_paths(&lock) {
        Ok(paths) => paths,
        Err(reason) => {
            result.findings.push(source_finding(reason, LOCK));
            return Ok(result);
        }
    };
    let mut total = files.values().map(Vec::len).sum::<usize>();
    let mut reads = stream::iter(paths.into_iter().filter(|path| !files.contains_key(path)))
        .map(|path| {
            let source = &source;
            async move {
                let read = client.frozen_blob(source, &path).await;
                (path, read)
            }
        })
        .buffer_unordered(8);
    let mut acquired = Vec::new();
    while let Some((path, read)) = reads.next().await {
        match read {
            Ok(Some(bytes)) => {
                total = total.saturating_add(bytes.len());
                if total > TOTAL_BYTES {
                    result
                        .findings
                        .push(source_finding("source_bounds_exceeded", LOCK));
                    return Ok(result);
                }
                acquired.push((path, bytes));
            }
            Ok(None) => {
                result.findings.push(finding("locked_input_missing", LOCK,
                    "Commit every input referenced by the environment lock and regenerate it with Ayni 0.16.2."));
                return Ok(result);
            }
            Err(reason @ ("source_bounds_exceeded" | "source_path_unsupported")) => {
                result.findings.push(source_finding(reason, &path));
                return Ok(result);
            }
            Err(reason) => return Err(reason),
        }
    }
    drop(reads);
    files.extend(acquired);
    tokio::task::spawn_blocking(move || {
        validate_inputs(&mut result, &files)?;
        Ok(result)
    })
    .await
    .map_err(|_| "inspection_unavailable")?
}

fn record_header(
    result: &mut InspectionResult,
    files: &mut BTreeMap<String, Vec<u8>>,
    path: &str,
    read: Result<Option<Vec<u8>>, &'static str>,
) -> Result<(), &'static str> {
    match read {
        Ok(Some(bytes)) => {
            files.insert(path.to_owned(), bytes);
        }
        Ok(None) => {}
        Err(reason @ ("source_bounds_exceeded" | "source_path_unsupported")) => {
            result.findings.push(source_finding(reason, path));
        }
        Err(reason) => return Err(reason),
    }
    Ok(())
}
fn header_rejected(result: &InspectionResult, path: &str) -> bool {
    result.findings.iter().any(|finding| {
        finding.path == path
            && matches!(
                finding.code.as_str(),
                "source_bounds_exceeded" | "source_path_unsupported"
            )
    })
}

fn inspect_headers(
    mut result: InspectionResult,
    files: &BTreeMap<String, Vec<u8>>,
) -> (InspectionResult, Option<EnvironmentLock>) {
    let policy = parse_policy(&mut result, files.get(POLICY).map(Vec::as_slice));
    let lock = parse_lock(&mut result, files.get(LOCK).map(Vec::as_slice));
    if let Some(lock) = &lock {
        summarize_lock(&mut result, lock);
    }
    if let Some(policy) = &policy {
        summarize_trust(&mut result, policy, lock.as_ref());
    }
    bound_summary(&mut result);
    (result, if policy.is_some() { lock } else { None })
}

fn parse_policy(result: &mut InspectionResult, bytes: Option<&[u8]>) -> Option<AyniPolicy> {
    match bytes {
        None if header_rejected(result, POLICY) => None,
        None => {
            result.findings.push(finding(
                "policy_missing",
                POLICY,
                "Commit .ayni.toml using Ayni 0.16.2, then inspect that commit.",
            ));
            None
        }
        Some(bytes) => {
            result.policy_digest = sha256_fingerprint(bytes);
            if let Some(policy) = std::str::from_utf8(bytes)
                .ok()
                .and_then(|text| AyniPolicy::parse(text).ok())
            {
                Some(policy)
            } else {
                result.findings.push(finding(
                    "policy_invalid",
                    POLICY,
                    "Repair the policy using Ayni 0.16.2 and commit the corrected file.",
                ));
                None
            }
        }
    }
}

fn parse_lock(result: &mut InspectionResult, bytes: Option<&[u8]>) -> Option<EnvironmentLock> {
    match bytes {
        None if header_rejected(result, LOCK) => None,
        None => {
            result.findings.push(finding(
                "lock_missing",
                LOCK,
                "Run ayni env lock with Ayni 0.16.2 and commit .ayni.lock and its input files.",
            ));
            None
        }
        Some(bytes) => {
            result.lock_digest = sha256_fingerprint(bytes);
            // Deserialize with Ayni's contract: schema, recipe, fingerprint and structural validation remain upstream-owned.
            if let Ok(lock) = serde_json::from_slice::<EnvironmentLock>(bytes) {
                Some(lock)
            } else {
                result.findings.push(finding("lock_invalid_or_unsupported", LOCK, "Regenerate and commit the lock with supported Ayni 0.16.2; the lock must have a valid schema, recipe and fingerprint."));
                None
            }
        }
    }
}

fn summarize_lock(result: &mut InspectionResult, lock: &EnvironmentLock) {
    result.lock_fingerprint = lock.fingerprint().into();
    result.ayni_version = lock.ayni_version().into();
    result.lock_schema = ayni_core::ENVIRONMENT_LOCK_SCHEMA_VERSION.into();
    let mut tools = BTreeSet::new();
    for tool in lock.tools() {
        tools.insert(format!("{}@{}", tool.tool, tool.version));
    }
    for target in lock.targets() {
        for tool in &target.runtimes {
            tools.insert(format!("{}@{}", tool.runtime, tool.version));
        }
        for tool in &target.signal_tools {
            tools.insert(format!("{}@{}", tool.tool, tool.version));
        }
        if let Some(tool) = &target.package_manager {
            tools.insert(format!("{}@{}", tool.family, tool.version));
        }
    }
    result.tools = tools.into_iter().collect();
    if lock.repository().contract_path != POLICY {
        result.findings.push(finding(
            "policy_path_unsupported",
            LOCK,
            "Bind the environment lock to the root .ayni.toml policy before runtime preparation.",
        ));
    }
    if lock.ayni_version() != "0.16.2" {
        result.findings.push(finding(
            "ayni_version_unsupported",
            LOCK,
            "Regenerate the lock with supported Ayni 0.16.2 before runtime preparation.",
        ));
    }
}

fn summarize_trust(
    result: &mut InspectionResult,
    policy: &AyniPolicy,
    lock: Option<&EnvironmentLock>,
) {
    match policy.environment_certificate_trust_policy() {
            Ok(trust) => {
                result.trust_key_ids = policy.environment.certificate.trusted_keys.keys().cloned().collect();
                if result.trust_key_ids.is_empty() {
                    result.findings.push(finding("trust_required", POLICY, "Configure reviewed public certificate trust keys in environment.certificate.trusted_keys, regenerate the lock and commit both files."));
                }
                if lock.is_some_and(|lock| lock.certificate_trust_policy().fingerprint != trust.fingerprint()) {
                    result.findings.push(finding("trust_fingerprint_mismatch", LOCK, "Review the public certificate trust policy and regenerate the lock to bind its exact trust fingerprint."));
                }
            }
            Err(_) => result.findings.push(finding("trust_invalid", POLICY, "Configure valid public certificate trust key identifiers and public keys with Ayni 0.16.2.")),
        }
}

fn bound_summary(result: &mut InspectionResult) {
    // Receipt summaries must fit the durable result budget even when valid source
    // contracts contain a very large number of tools or long exact selectors.
    let oversized = result.tools.len() > 128
        || result.trust_key_ids.len() > 128
        || result
            .tools
            .iter()
            .chain(&result.trust_key_ids)
            .any(|v| v.len() > 128)
        || result.ayni_version.len() > 128;
    if oversized {
        result.tools.clear();
        result.trust_key_ids.clear();
        if result.ayni_version.len() > 128 {
            result.ayni_version.clear();
        }
        result.findings.push(finding("inspection_metadata_bounds_exceeded", LOCK,
            "Reduce the policy and lock summary to 128 tools and 128 trust keys, with identifiers and versions of at most 128 bytes, then retry. Tool and trust summaries were omitted."));
    }
}

fn locked_paths(lock: &EnvironmentLock) -> Result<BTreeSet<String>, &'static str> {
    let mut paths = BTreeSet::from([
        POLICY.to_owned(),
        LOCK.to_owned(),
        lock.repository().contract_path.clone(),
    ]);
    for target in lock.targets() {
        for source in target
            .runtimes
            .iter()
            .map(|v| &v.source)
            .chain(target.package_manager.iter().map(|v| &v.source))
            .chain(target.signal_tools.iter().map(|v| &v.source))
            .chain(target.dependency_locks.iter().map(|v| &v.source))
        {
            if source.digest.is_some() {
                paths.insert(source.path.clone());
            }
        }
        for dependency in &target.dependency_locks {
            paths.insert(dependency.path.clone());
        }
    }
    for source in lock
        .tools()
        .iter()
        .map(|v| &v.source)
        .chain(lock.debian_packages().iter().map(|v| &v.source))
    {
        if source.digest.is_some() {
            paths.insert(source.path.clone());
        }
    }
    if paths.len() > 256 {
        return Err("source_bounds_exceeded");
    }
    if paths.iter().any(|v| !safe_path(v)) {
        return Err("source_path_unsupported");
    }
    // A file cannot also be an ancestor directory of another materialized file.
    for path in &paths {
        if path
            .split('/')
            .scan(String::new(), |prefix, part| {
                if !prefix.is_empty() {
                    prefix.push('/');
                }
                prefix.push_str(part);
                Some(prefix.clone())
            })
            .any(|prefix| prefix != *path && paths.contains(&prefix))
        {
            return Err("source_path_unsupported");
        }
    }
    Ok(paths)
}
fn validate_inputs(
    result: &mut InspectionResult,
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<(), &'static str> {
    let root = tempfile::tempdir().map_err(|_| "inspection_unavailable")?;
    for (path, bytes) in files {
        if !safe_path(path) {
            return Err("source_path_unsupported");
        }
        let path = root.path().join(path);
        fs::create_dir_all(path.parent().ok_or("inspection_unavailable")?)
            .map_err(|_| "inspection_unavailable")?;
        fs::write(path, bytes).map_err(|_| "inspection_unavailable")?;
    }
    let canonical_root = root
        .path()
        .canonicalize()
        .map_err(|_| "inspection_unavailable")?;
    match ayni_environment::read_lock(&canonical_root) {
        Ok(lock) => {
            // Ayni's reader checks target inputs. Also bind generic tool/package
            // sources to their declared bytes without reproducing adapter logic.
            if lock
                .tools()
                .iter()
                .map(|tool| &tool.source)
                .chain(lock.debian_packages().iter().map(|package| &package.source))
                .any(|source| !source_digest_matches(source, files))
            {
                result.findings.push(finding("locked_input_digest_mismatch", LOCK,
                    "Regenerate the lock with Ayni 0.16.2 from the committed generic tool and package source files; their bytes must match the declared digests."));
            }
        }
        Err(_) => {
            result.findings.push(finding("lock_inputs_or_platform_invalid", LOCK,
                "Regenerate the lock from these committed input bytes with Ayni 0.16.2, and include the control-plane host architecture in its platforms. Locked input digests and host architecture must both validate."));
        }
    }
    Ok(())
}

fn source_digest_matches(
    source: &ayni_core::LockedRequirementSource,
    files: &BTreeMap<String, Vec<u8>>,
) -> bool {
    source.digest.as_ref().is_none_or(|expected| {
        files
            .get(&source.path)
            .is_some_and(|bytes| sha256_fingerprint(bytes) == *expected)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> std::io::Result<BTreeMap<String, Vec<u8>>> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../dev/fixtures/task-quality/source");
        [
            POLICY,
            LOCK,
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
        ]
        .into_iter()
        .map(|path| fs::read(root.join(path)).map(|bytes| (path.to_owned(), bytes)))
        .collect()
    }
    #[test]
    fn real_ayni_fixture_validates_exact_inputs_and_never_claims_readiness()
    -> Result<(), Box<dyn std::error::Error>> {
        let files = fixture()?;
        let (mut result, lock) = inspect_headers(empty_result(), &files);
        let lock = lock.ok_or("official fixture lock missing")?;
        assert_eq!(result.ayni_version, "0.16.2");
        assert_eq!(result.lock_schema, "0.9.0");
        assert_eq!(result.lock_fingerprint, lock.fingerprint());
        assert!(result.tools.contains(&"rust@1.94.0".into()));
        assert!(result.findings.iter().any(|v| v.code == "trust_required"));
        assert!(
            result
                .findings
                .iter()
                .any(|v| v.code == "adapter_discovery_pending")
        );
        assert!(
            result
                .findings
                .iter()
                .any(|v| v.code == "runtime_verification_pending")
        );
        assert_eq!(locked_paths(&lock)?, files.keys().cloned().collect());
        validate_inputs(&mut result, &files)?;
        assert!(
            !result
                .findings
                .iter()
                .any(|v| v.code == "lock_inputs_or_platform_invalid")
        );
        let mut changed = files;
        changed
            .get_mut("Cargo.toml")
            .ok_or("fixture manifest missing")?
            .push(b'\n');
        validate_inputs(&mut result, &changed)?;
        assert!(
            result
                .findings
                .iter()
                .any(|v| v.code == "lock_inputs_or_platform_invalid")
        );
        Ok(())
    }
    #[test]
    fn generic_tool_and_package_sources_require_exact_frozen_bytes()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut files = BTreeMap::from([("mise.toml".into(), b"original".to_vec())]);
        let source = ayni_core::LockedRequirementSource {
            kind: "mise_toml".into(),
            path: "mise.toml".into(),
            digest: Some(sha256_fingerprint(b"original")),
            confidence: ayni_core::RequirementConfidence::Exact,
        };
        assert!(source_digest_matches(&source, &files));
        files
            .get_mut("mise.toml")
            .ok_or("fixture tool config missing")?
            .push(b'!');
        assert!(!source_digest_matches(&source, &files));
        files.clear();
        assert!(!source_digest_matches(&source, &files));
        Ok(())
    }
    #[test]
    fn rejected_policy_and_lock_have_distinct_findings_without_false_missing_inputs()
    -> Result<(), Box<dyn std::error::Error>> {
        for reason in ["source_bounds_exceeded", "source_path_unsupported"] {
            let mut result = empty_result();
            let mut files = BTreeMap::new();
            for path in [POLICY, LOCK] {
                record_header(&mut result, &mut files, path, Err(reason))?;
            }
            let (result, lock) = inspect_headers(result, &files);
            assert!(lock.is_none());
            let rejected = result
                .findings
                .iter()
                .filter(|finding| finding.code == reason)
                .collect::<Vec<_>>();
            assert_eq!(rejected.len(), 2);
            assert_eq!(rejected[0].path, POLICY);
            assert_eq!(rejected[1].path, LOCK);
            assert!(
                !result
                    .findings
                    .iter()
                    .any(|finding| finding.code.ends_with("_missing"))
            );
            let keys = result
                .findings
                .iter()
                .map(|finding| (&finding.code, &finding.path))
                .collect::<BTreeSet<_>>();
            assert_eq!(keys.len(), result.findings.len());
        }
        Ok(())
    }
    #[test]
    fn missing_and_malformed_inputs_produce_actionable_completed_findings() {
        let (result, lock) = inspect_headers(empty_result(), &BTreeMap::new());
        assert!(lock.is_none());
        assert!(result.findings.iter().any(|v| v.code == "policy_missing"));
        assert!(result.findings.iter().any(|v| v.code == "lock_missing"));
        let files = BTreeMap::from([
            (POLICY.into(), b"not a policy".to_vec()),
            (LOCK.into(), b"{}".to_vec()),
        ]);
        let (result, lock) = inspect_headers(empty_result(), &files);
        assert!(lock.is_none());
        assert!(result.findings.iter().any(|v| v.code == "policy_invalid"));
        assert!(
            result
                .findings
                .iter()
                .any(|v| v.code == "lock_invalid_or_unsupported")
        );
        assert!(result.findings.iter().all(|v| !v.action.is_empty()));
        assert!(result.policy_digest.starts_with("sha256:"));
        assert!(result.lock_digest.starts_with("sha256:"));
    }
    #[test]
    fn official_lock_deserializer_rejects_tampered_fingerprint_and_future_schema()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut files = fixture()?;
        let original = files[LOCK].clone();
        for (field, value) in [
            ("schema_version", "99.0.0"),
            (
                "fingerprint",
                "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            ),
        ] {
            let mut lock: serde_json::Value = serde_json::from_slice(&original)?;
            lock[field] = value.into();
            files.insert(LOCK.into(), serde_json::to_vec(&lock)?);
            let (result, lock) = inspect_headers(empty_result(), &files);
            assert!(lock.is_none());
            assert!(
                result
                    .findings
                    .iter()
                    .any(|v| v.code == "lock_invalid_or_unsupported")
            );
        }
        Ok(())
    }
}
