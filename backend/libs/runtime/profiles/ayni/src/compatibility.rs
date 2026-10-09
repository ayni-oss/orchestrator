//! Explicit admission catalog. Repository locks select contracts; deployments do not.
use serde_json::Value;

/// A CLI/artifact contract tested by this integration.
#[derive(Clone, Copy)]
pub struct AyniContract {
    pub version: &'static str,
    pub resolver_version: &'static str,
    pub lock_schema: &'static str,
    pub recipe_version: &'static str,
    pub metadata_schema: &'static str,
    pub signals_schema: &'static str,
    pub builder_reference: &'static str,
    pub platforms: &'static [&'static str],
}

const SUPPORTED: &[AyniContract] = &[AyniContract {
    version: "0.15.0",
    resolver_version: "2026.6.14",
    lock_schema: "0.9.0",
    recipe_version: "2",
    metadata_schema: "2",
    signals_schema: "0.5.0",
    builder_reference: "ghcr.io/ayni-oss/ayni-builder:0.15.0",
    platforms: &["linux/amd64", "linux/arm64"],
}];

/// Select exactly an admitted release, never a compatible-looking version range.
/// # Errors
/// Unsupported versions require an explicit repository upgrade.
pub fn contract(version: &str) -> Result<&'static AyniContract, &'static str> {
    lookup(SUPPORTED, version)
}

fn lookup<'a>(
    catalog: &'a [AyniContract],
    version: &str,
) -> Result<&'a AyniContract, &'static str> {
    catalog
        .iter()
        .find(|item| item.version == version)
        .ok_or("runtime_version_unsupported")
}

/// Minimal routing identity; authoritative policy and lock interpretation stays in Ayni.
pub struct LockEnvelope {
    pub contract_version: String,
    pub fingerprint: String,
    pub platform: String,
}

/// Bound and route an untrusted lock before invoking its approved CLI.
/// # Errors
/// Reject malformed envelopes, unsupported versions and unavailable platforms.
pub fn validate_lock_envelope(bytes: &[u8], platform: &str) -> Result<LockEnvelope, &'static str> {
    validate_envelope(SUPPORTED, bytes, platform)
}

fn validate_envelope(
    catalog: &[AyniContract],
    bytes: &[u8],
    platform: &str,
) -> Result<LockEnvelope, &'static str> {
    if bytes.len() > crate::managed_runtime::MAX_METADATA_BYTES {
        return Err("runtime_lock_invalid");
    }
    let lock: Value = serde_json::from_slice(bytes).map_err(|_| "runtime_lock_invalid")?;
    let version = lock["ayni_version"]
        .as_str()
        .ok_or("runtime_lock_invalid")?;
    let supported = lookup(catalog, version)?;
    if lock["schema_version"] != supported.lock_schema
        || lock["recipe_version"] != supported.recipe_version
    {
        return Err("runtime_lock_contract_unsupported");
    }
    if lock["mise_version"] != supported.resolver_version {
        return Err("runtime_resolver_unsupported");
    }
    let fingerprint = lock["fingerprint"]
        .as_str()
        .filter(|value| digest(value))
        .ok_or("runtime_lock_invalid")?;
    let (os, architecture) = platform.split_once('/').ok_or("runtime_platform_invalid")?;
    if !supported.platforms.contains(&platform)
        || !lock["platforms"].as_array().is_some_and(|items| {
            items.iter().any(|item| {
                item["os"] == os && item["architecture"] == architecture && item["libc"] == "glibc"
            })
        })
    {
        return Err("runtime_platform_mismatch");
    }
    Ok(LockEnvelope {
        contract_version: version.into(),
        fingerprint: fingerprint.into(),
        platform: platform.into(),
    })
}

/// Validate the controller-resolved official manifest reference before execution.
/// # Errors
/// Reject mutable tags and repository-selected substitutes.
pub fn validate_builder(image: &str) -> Result<(), &'static str> {
    let digest = image
        .strip_prefix("ghcr.io/ayni-oss/ayni-builder@")
        .ok_or("runtime_builder_unapproved")?;
    if !self::digest(digest) {
        return Err("runtime_builder_unpinned");
    }
    Ok(())
}

fn digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|value| {
        value.len() == 64
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn admission_is_exact_and_platform_specific() {
        let mut lock = serde_json::json!({"ayni_version":"0.15.0","mise_version":"2026.6.14","schema_version":"0.9.0","recipe_version":"2","fingerprint":format!("sha256:{}","a".repeat(64)),"platforms":[{"os":"linux","architecture":"amd64","libc":"glibc"}]});
        assert!(validate_lock_envelope(lock.to_string().as_bytes(), "linux/amd64").is_ok());
        assert!(validate_lock_envelope(lock.to_string().as_bytes(), "linux/arm64").is_err());
        for version in ["0.14.5", "0.15.1", "latest"] {
            lock["ayni_version"] = version.into();
            assert!(validate_lock_envelope(lock.to_string().as_bytes(), "linux/amd64").is_err());
        }
        assert!(validate_lock_envelope(&vec![b' '; 1_048_577], "linux/amd64").is_err());
        assert!(
            validate_builder(&format!(
                "ghcr.io/ayni-oss/ayni-builder@sha256:{}",
                "a".repeat(64)
            ))
            .is_ok()
        );
        assert!(validate_builder("ghcr.io/ayni-oss/ayni-builder:0.15.0").is_err());
        assert!(
            validate_builder(&format!("ghcr.io/other/builder@sha256:{}", "a".repeat(64))).is_err()
        );
    }

    #[test]
    fn repository_versions_resolve_independently_without_deployment_state() {
        static CATALOG: [AyniContract; 2] = [
            SUPPORTED[0],
            AyniContract {
                version: "test-only-2",
                builder_reference: "ghcr.io/ayni-oss/ayni-builder:test-only-2",
                ..SUPPORTED[0]
            },
        ];
        std::thread::scope(|scope| {
            for version in ["0.15.0", "test-only-2"] {
                scope.spawn(move || {
                    let mut lock = serde_json::json!({"ayni_version":version,"mise_version":"2026.6.14","schema_version":"0.9.0","recipe_version":"2","fingerprint":format!("sha256:{}","a".repeat(64)),"platforms":[{"os":"linux","architecture":"amd64","libc":"glibc"}]});
                    let admitted = validate_envelope(&CATALOG, lock.to_string().as_bytes(), "linux/amd64");
                    assert!(admitted.is_ok_and(|value| value.contract_version == version));
                    lock["recipe_version"] = "1".into();
                    assert!(validate_envelope(&CATALOG, lock.to_string().as_bytes(), "linux/amd64").is_err());
                    lock["recipe_version"] = "2".into();
                    lock["schema_version"] = "0.10.0".into();
                    assert!(validate_envelope(&CATALOG, lock.to_string().as_bytes(), "linux/amd64").is_err());
                });
            }
        });
        assert!(contract("test-only-2").is_err());
    }
}
