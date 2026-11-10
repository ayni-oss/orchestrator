use ayni_core::{
    AyniPolicy, DebianPackageRequirement, EnvironmentLock, EnvironmentPlan,
    LockedRequirementSource, LockedTargetEnvironment, MiseToolRequirement, ProvisioningSupport,
    RepositoryIdentity, RequirementSource, RuntimeRequirement, SignalToolRequirement,
    TargetEnvironment, VersionRequirement, sha256_fingerprint,
};
use std::collections::BTreeMap;
use toml_edit::{DocumentMut, Item, Table, TableLike, value};

const INVALID: &str =
    "The reviewed policy and lock could not be updated without changing unrelated requirements.";

pub(super) fn update(
    policy: &[u8],
    lock: &[u8],
    key: &str,
    public_key: &str,
) -> Result<(String, String), &'static str> {
    let policy = std::str::from_utf8(policy).map_err(|_| INVALID)?;
    let existing: EnvironmentLock = serde_json::from_slice(lock).map_err(|_| INVALID)?;
    let previous = AyniPolicy::parse(policy).map_err(|_| INVALID)?;
    let mut document = policy.parse::<DocumentMut>().map_err(|_| INVALID)?;
    let mut table: &mut dyn TableLike = document.as_table_mut();
    for component in ["environment", "certificate", "trusted_keys"] {
        if !table.contains_key(component) {
            table.insert(component, Item::Table(Table::new()));
        }
        table = table
            .get_mut(component)
            .and_then(Item::as_table_like_mut)
            .ok_or(INVALID)?;
    }
    table.insert(key, value(public_key));
    let updated_policy = document.to_string();
    let updated = AyniPolicy::parse(&updated_policy).map_err(|_| INVALID)?;
    let mut expected = previous.environment.certificate.trusted_keys.clone();
    if expected.insert(key.into(), public_key.into()).as_deref() == Some(public_key)
        || updated.environment.certificate.trusted_keys != expected
    {
        return Err(INVALID);
    }
    // Verify the original round trip before changing the policy and source digest.
    // Ayni remains responsible for lock normalization and fingerprint generation.
    if project(&existing, &previous, policy)? != existing {
        return Err(INVALID);
    }
    let lock = project(&existing, &updated, &updated_policy)?
        .canonical_json()
        .map_err(|_| INVALID)?;
    Ok((updated_policy, lock))
}

fn source(
    value: &LockedRequirementSource,
    digests: &mut BTreeMap<String, String>,
) -> RequirementSource {
    if let Some(digest) = &value.digest {
        digests.insert(value.path.clone(), digest.clone());
    }
    RequirementSource {
        kind: value.kind.clone(),
        path: value.path.clone(),
        confidence: value.confidence,
        detail: None,
    }
}

fn exact(version: &str) -> VersionRequirement {
    VersionRequirement::Exact {
        version: version.into(),
    }
}

fn target_requirements(
    target: &LockedTargetEnvironment,
    digests: &mut BTreeMap<String, String>,
) -> TargetEnvironment {
    TargetEnvironment {
        target: target.target.clone(),
        workspace: None,
        package: None,
        runtimes: target
            .runtimes
            .iter()
            .map(|runtime| RuntimeRequirement {
                runtime: runtime.runtime.clone(),
                version: exact(&runtime.version),
                components: runtime.components.clone(),
                targets: runtime.targets.clone(),
                source: source(&runtime.source, digests),
            })
            .collect(),
        package_manager: target.package_manager.as_ref().map(|manager| {
            ayni_core::PackageManagerRequirement {
                family: manager.family.clone(),
                version: exact(&manager.version),
                ownership_root: manager.ownership_root.clone(),
                source: source(&manager.source, digests),
            }
        }),
        signal_tools: target
            .signal_tools
            .iter()
            .map(|tool| SignalToolRequirement {
                tool: tool.tool.clone(),
                version_authority: tool.version_authority,
                version: exact(&tool.version),
                provider: tool.provider.clone(),
                scope: tool.scope,
                signals: tool.signals.clone(),
                supported_platforms: vec![],
                provisioning: ProvisioningSupport::LockedOffline,
                modifies_checkout: false,
                source: source(&tool.source, digests),
            })
            .collect(),
        system_requirements: vec![],
        dependency_locks: target
            .dependency_locks
            .iter()
            .map(|dependency| ayni_core::DependencyLockRequirement {
                path: dependency.path.clone(),
                digest: dependency.digest.clone(),
                owner_root: dependency.owner_root.clone(),
                source: source(&dependency.source, digests),
            })
            .collect(),
    }
}

fn project(
    lock: &EnvironmentLock,
    policy: &AyniPolicy,
    policy_text: &str,
) -> Result<EnvironmentLock, &'static str> {
    let mut digests = BTreeMap::new();
    let targets = lock
        .targets()
        .iter()
        .map(|target| target_requirements(target, &mut digests))
        .collect();
    let tools = lock
        .tools()
        .iter()
        .map(|tool| MiseToolRequirement {
            tool: tool.tool.clone(),
            version: exact(&tool.version),
            source: source(&tool.source, &mut digests),
        })
        .collect();
    let packages = lock
        .debian_packages()
        .iter()
        .map(|package| DebianPackageRequirement {
            package: package.package.clone(),
            source: source(&package.source, &mut digests),
        })
        .collect();
    let contract_path = &lock.repository().contract_path;
    if contract_path != ".ayni.toml" {
        return Err(INVALID);
    }
    let contract_digest = sha256_fingerprint(policy_text);
    if digests.contains_key(contract_path) {
        digests.insert(contract_path.clone(), contract_digest.clone());
    }
    let plan = EnvironmentPlan::new(
        RepositoryIdentity {
            name: "repository".into(),
            contract_digest,
        },
        lock.platforms().to_vec(),
        targets,
        vec![],
        vec![],
    )
    .and_then(|plan| plan.with_tools(tools))
    .and_then(|plan| plan.with_debian_packages(packages))
    .and_then(|plan| plan.with_capabilities(lock.capabilities()))
    .and_then(|plan| plan.with_resource_limits(lock.resource_limits()))
    .and_then(EnvironmentPlan::resolve)
    .map_err(|_| INVALID)?;
    let trust = policy
        .environment_certificate_trust_policy()
        .map_err(|_| INVALID)?;
    EnvironmentLock::from_resolved_plan(
        &plan,
        lock.ayni_version(),
        lock.mise_version(),
        lock.provisioning_base().clone(),
        &trust,
        contract_path,
        &digests,
    )
    .map_err(|_| INVALID)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_trust_using_the_existing_resolved_inputs() -> Result<(), Box<dyn std::error::Error>>
    {
        let policy = include_bytes!(
            "../../../../../../../dev/fixtures/runtime-preparation/source/.ayni.toml"
        );
        let lock = include_bytes!(
            "../../../../../../../dev/fixtures/runtime-preparation/source/.ayni.lock"
        );
        let public_key = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
        let (policy, updated) = update(policy, lock, "new-key", public_key)?;
        let trust = AyniPolicy::parse(&policy)?.environment_certificate_trust_policy()?;
        let old: EnvironmentLock = serde_json::from_slice(lock)?;
        let new: EnvironmentLock = serde_json::from_str(&updated)?;
        assert_eq!(
            new.certificate_trust_policy().fingerprint,
            trust.fingerprint()
        );
        assert_ne!(old.fingerprint(), new.fingerprint());
        assert_eq!(old.targets(), new.targets());
        assert!(policy.contains("runtime-fixture"));
        assert!(policy.contains("new-key"));
        Ok(())
    }
}
