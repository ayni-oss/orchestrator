//! Public-policy preparation through the pinned Ayni CLI, without secret authority.
use crate::operations::runtime::{command, git, validate_source};
use ayni_orchestrator_profile_ayni::{compatibility, runtime_execution::ExecutionRequest};
use ayni_orchestrator_runtime::runtime_execution::read_bounded;
use serde_json::Value;
use std::{env, fs, io, path::Path, time::Duration};

/// Validate the frozen repository and regenerate its public trust inputs.
/// # Errors
/// Rejects authority leaks, invalid policy, changed source, or nonrepeatable lock output.
pub async fn run(request: &ExecutionRequest, workspace: &Path) -> io::Result<()> {
    validate_source(request)?;
    super::validate_cli_version(&request.expected.ayni_version, workspace).await?;
    if git(workspace, &["rev-parse", "HEAD"])? != request.revision {
        return Err(io::Error::other("trust source mismatch"));
    }
    let (key, public) = trust_identity()?;
    let original_policy = read_bounded(&workspace.join(".ayni.toml"), 1_048_576)?;
    let original_lock = read_bounded(&workspace.join(".ayni.lock"), 1_048_576)?;
    let old: Value = serde_json::from_slice(&original_lock).map_err(io::Error::other)?;
    let envelope =
        compatibility::validate_lock_envelope(&original_lock, &request.expected.platform)
            .map_err(io::Error::other)?;
    if envelope.contract_version != request.expected.ayni_version
        || envelope.fingerprint != request.expected.lock_fingerprint
    {
        return Err(io::Error::other("runtime_lock_mismatch"));
    }
    // Regeneration with the matching CLI validates policy, sources and lock together.
    // Reject stale inputs before making any trust change.
    let base = format!(
        "{}@{}",
        old["provisioning_base"]["reference"]
            .as_str()
            .ok_or_else(|| io::Error::other("base missing"))?,
        old["provisioning_base"]["digest"]
            .as_str()
            .ok_or_else(|| io::Error::other("base digest missing"))?
    );
    cli(workspace, &["env", "lock", "--base", &base]).await?;
    let regenerated: Value =
        serde_json::from_slice(&read_bounded(&workspace.join(".ayni.lock"), 1_048_576)?)
            .map_err(io::Error::other)?;
    if regenerated != old {
        return Err(io::Error::other("repository lock is stale"));
    }
    let mut policy: toml_edit::DocumentMut = std::str::from_utf8(&original_policy)
        .map_err(io::Error::other)?
        .parse()
        .map_err(io::Error::other)?;
    if policy
        .get("environment")
        .and_then(|item| item.get("certificate"))
        .and_then(|item| item.get("trusted_keys"))
        .and_then(|item| item.get(&key))
        .and_then(toml_edit::Item::as_str)
        == Some(public.as_str())
    {
        emit_result(serde_json::json!({
            "revision":request.revision,
            "policy":std::str::from_utf8(&original_policy).map_err(io::Error::other)?,
            "lock":std::str::from_utf8(&original_lock).map_err(io::Error::other)?
        }))
        .await?;
        return Ok(());
    }
    policy["environment"]["certificate"]["trusted_keys"][&key] = toml_edit::value(&public);
    let policy = policy.to_string();
    fs::write(workspace.join(".ayni.toml"), &policy)?;
    cli(workspace, &["env", "lock", "--base", &base]).await?;
    let first = read_bounded(&workspace.join(".ayni.lock"), 1_048_576)?;
    cli(workspace, &["env", "lock", "--base", &base]).await?;
    let second = read_bounded(&workspace.join(".ayni.lock"), 1_048_576)?;
    validate_lock(&old, &first, &second)?;
    let expected = request.revision.clone();
    emit_result(serde_json::json!({"revision":expected,"policy":policy,"lock":String::from_utf8(first).map_err(io::Error::other)?})).await?;
    Ok(())
}

fn trust_identity() -> io::Result<(String, String)> {
    for variable in [
        "AYNI_ENV_CERTIFICATE_SIGNING_KEY",
        "DOCKER_HOST",
        "SOURCE_TOKEN",
    ] {
        if env::var_os(variable).is_some() {
            return Err(io::Error::other("trust job authority invalid"));
        }
    }
    let key = env::var("CERTIFICATE_KEY_ID").map_err(io::Error::other)?;
    let public = env::var("CERTIFICATE_PUBLIC_KEY").map_err(io::Error::other)?;
    if key.is_empty()
        || key == "unsigned"
        || key.len() > 128
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        || public.len() != 64
        || !public
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
    {
        return Err(io::Error::other("trust identity invalid"));
    }
    Ok((key, public))
}

fn validate_lock(old: &Value, first: &[u8], second: &[u8]) -> io::Result<()> {
    if first != second {
        return Err(io::Error::other("lock generation is not repeatable"));
    }
    let new: Value = serde_json::from_slice(first).map_err(io::Error::other)?;
    let mut before = old.clone();
    let mut after = new;
    // Only the policy digest, trust-policy fingerprint and derived lock identity may change.
    for value in [&mut before, &mut after] {
        let object = value
            .as_object_mut()
            .ok_or_else(|| io::Error::other("invalid lock"))?;
        object.remove("fingerprint");
        if let Some(repository) = object.get_mut("repository").and_then(Value::as_object_mut) {
            repository.remove("contract_digest");
        }
        if let Some(trust) = object
            .get_mut("certificate_trust_policy")
            .and_then(Value::as_object_mut)
        {
            trust.remove("fingerprint");
        }
    }
    if before != after {
        return Err(io::Error::other("immutable environment inputs changed"));
    }
    Ok(())
}

async fn cli(workspace: &Path, args: &[&str]) -> io::Result<()> {
    let mut process = command("/usr/local/bin/ayni", workspace);
    process
        .env("MISE_DATA_DIR", "/scratch/mise-data")
        .env("MISE_CONFIG_DIR", "/scratch/mise-config")
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let status = crate::operations::process::status(
        &mut tokio::process::Command::from(process),
        Duration::from_secs(600),
    )
    .await?;
    if !status.success() {
        return Err(io::Error::other("Ayni trust lock validation failed"));
    }
    Ok(())
}

async fn emit_result(result: Value) -> io::Result<()> {
    if crate::operation::enabled() {
        let field = |name| {
            result[name]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| io::Error::other("runtime_result_invalid"))
        };
        crate::operation::complete(
            ayni_orchestrator_runtime::operation::OperationOutcome::Trust {
                revision: field("revision")?,
                policy: field("policy")?,
                lock: field("lock")?,
            },
        )
        .await
    } else {
        println!("AYNI_TRUST_RESULT={result}");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::validate_lock;
    use serde_json::json;

    #[test]
    fn public_trust_updates_preserve_pinned_environment_inputs()
    -> Result<(), Box<dyn std::error::Error>> {
        let old = json!({"provisioning_base":{"reference":"registry.test/base","digest":"sha256:123"},"targets":[],"recipe_version":"2","mise_version":"2026.6.14","platforms":["linux/amd64"],"ayni_version":"0.15.0","certificate_trust_policy":{"fingerprint":"old"}});
        let mut updated = old.clone();
        updated["certificate_trust_policy"]["fingerprint"] = json!("new");
        let first = serde_json::to_vec(&updated)?;
        validate_lock(&old, &first, &first)?;
        assert!(validate_lock(&old, &first, b"{}").is_err());
        for field in [
            "provisioning_base",
            "targets",
            "recipe_version",
            "mise_version",
            "platforms",
            "ayni_version",
            "tools",
            "debian_packages",
            "capabilities",
            "resources",
            "unknown_extension",
        ] {
            let mut changed = updated.clone();
            changed[field] = json!("changed");
            let output = serde_json::to_vec(&changed)?;
            assert!(validate_lock(&old, &output, &output).is_err(), "{field}");
        }
        let mut changed = updated;
        changed["certificate_trust_policy"]["schema_version"] = json!("99");
        let output = serde_json::to_vec(&changed)?;
        assert!(validate_lock(&old, &output, &output).is_err());
        Ok(())
    }
}
