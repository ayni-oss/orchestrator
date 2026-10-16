//! Reviewed lock proposals use the admitted runtime and never certify a future image.
use super::{quality, recovery, sessions::Context};
use crate::operations::runtime::{git, git_bytes};
use ayni_orchestrator_profile_ayni::{compatibility, managed_runtime};
use ayni_orchestrator_runtime::{coding::QualityResult, runtime_execution::read_bounded};
use serde_json::{Value, json};
use std::{io, time::Duration};

pub(super) const UNAVAILABLE: &str = "Local dependency preparation and proposed-source quality checks are unavailable: the pinned Ayni CLI cannot prepare changed dependencies in the admitted baseline runtime. No future runtime image has been built or certified. Repository-owner review and post-merge runtime reconciliation are required.";

pub(super) fn validate_policy(context: &Context) -> io::Result<()> {
    let baseline = git_bytes(
        &context.workspace,
        &[
            "show",
            &format!("{}:.ayni.toml", context.spec.execution.revision),
        ],
    )?;
    let policy = read_bounded(
        &context.workspace.join(".ayni.toml"),
        managed_runtime::MAX_METADATA_BYTES,
    )?;
    if policy.as_slice() != baseline.as_slice() {
        return Err(io::Error::other("protected policy changed"));
    }
    Ok(())
}

/// Regenerate via the exact supported CLI, retaining its baseline provisioning base.
/// Changed locks cannot use the baseline certificate as proposed-environment evidence.
pub(super) async fn prepare(
    context: &Context,
    deadline: Duration,
) -> io::Result<Option<QualityResult>> {
    prepare_with(context, deadline, &NativeRunner).await
}

trait LockRunner: Sync {
    fn generate(
        &self,
        context: &Context,
        base: &str,
        deadline: Duration,
    ) -> impl std::future::Future<Output = io::Result<bool>> + Send;
}
struct NativeRunner;
impl LockRunner for NativeRunner {
    async fn generate(
        &self,
        context: &Context,
        base: &str,
        deadline: Duration,
    ) -> io::Result<bool> {
        let mut process =
            crate::operations::sandbox::command("/usr/local/bin/ayni", &context.workspace)?;
        process.args(["env", "lock", "--base", base]);
        process
            .env("MISE_DATA_DIR", "/scratch/mise-data")
            .env("MISE_CONFIG_DIR", "/scratch/mise-config");
        Ok(crate::operations::process::status(
            &mut tokio::process::Command::from(process),
            deadline,
        )
        .await?
        .success())
    }
}

async fn prepare_with(
    context: &Context,
    deadline: Duration,
    runner: &impl LockRunner,
) -> io::Result<Option<QualityResult>> {
    if context.spec.constraints.environment_change.is_none() {
        return Ok(None);
    }
    validate_policy(context)?;
    super::commits::changed_paths(&context.spec, &context.workspace)?;
    let baseline = git_bytes(
        &context.workspace,
        &[
            "show",
            &format!("{}:.ayni.lock", context.spec.execution.revision),
        ],
    )?;
    let old: Value = serde_json::from_slice(&baseline).map_err(io::Error::other)?;
    let base = baseline_base(&old)?;
    let started = std::time::Instant::now();
    let generated = runner.generate(context, &base, deadline).await;
    validate_policy(context)?;
    if generated.as_ref().is_err() || generated.as_ref().is_ok_and(|success| !success) {
        // An unavailable generator cannot authorize an agent-written derived lock.
        recovery::atomic_write(&context.workspace.join(".ayni.lock"), &baseline)?;
    }
    let proposed = read_bounded(
        &context.workspace.join(".ayni.lock"),
        managed_runtime::MAX_METADATA_BYTES,
    )?;
    let unavailable =
        generated.as_ref().is_err() || generated.as_ref().is_ok_and(|success| !success);
    if !unavailable {
        let valid = compatibility::validate_lock_envelope(
            &proposed,
            &context.spec.execution.expected.platform,
        )
        .is_ok_and(|envelope| {
            envelope.contract_version == context.spec.execution.expected.ayni_version
        });
        if !valid {
            recovery::atomic_write(&context.workspace.join(".ayni.lock"), &baseline)?;
            return Err(io::Error::other(
                "protected generated lock contract is invalid",
            ));
        }
    }
    let changed = serde_json::from_slice::<Value>(&proposed).ok().as_ref() != Some(&old);
    if !changed && !unavailable {
        let path = context.output.join("evidence/environment-change.json");
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        return Ok(None);
    }
    let reason = if unavailable {
        "Supported ayni env lock regeneration failed or timed out; the baseline lock was restored and local dependency preparation is unavailable. No runtime image build was attempted."
    } else {
        UNAVAILABLE
    };
    recovery::write_json(
        &context.output.join("evidence/environment-change.json"),
        &json!({
            "baseline_revision":context.spec.execution.revision,
            "baseline_runtime_digest":context.spec.runtime_digest,
            "baseline_runtime":context.spec.execution.expected,
            "baseline_lock_digest":ayni_orchestrator_domain::digest::sha256(&baseline),
            "baseline_policy_digest":ayni_orchestrator_domain::digest::sha256(&read_bounded(&context.workspace.join(".ayni.toml"), managed_runtime::MAX_METADATA_BYTES)?),
            "proposed_lock_digest":ayni_orchestrator_domain::digest::sha256(&proposed),
            "source_commit":git(&context.workspace, &["rev-parse", "HEAD"])?,
            "dependency_paths":context.spec.constraints.environment_change.as_ref().map(|scope| &scope.dependency_paths),
            "dependency_inputs_digest":dependency_inputs_digest(context)?,
            "lock_regeneration":if unavailable {"unavailable"} else {"succeeded"},
            "local_dependency_preparation":"unavailable",
            "proposed_runtime_certification":"not_attempted",
            "elapsed_seconds":started.elapsed().as_secs(),
            "finding":reason,
        }),
    )?;
    Ok(Some(quality::unavailable(reason)))
}

pub(super) fn freeze(context: &Context) -> io::Result<()> {
    let path = context.output.join("evidence/environment-change.json");
    validate_proposal(context)?;
    if path.exists() {
        let mut evidence: Value = recovery::read_json(&path)?;
        evidence["source_commit"] = git(&context.workspace, &["rev-parse", "HEAD"])?.into();
        recovery::write_json(&path, &evidence)?;
    }
    Ok(())
}

pub(super) fn validate_retained(context: &Context) -> io::Result<()> {
    validate_proposal(context)?;
    let path = context.output.join("evidence/environment-change.json");
    if !path.exists() {
        return Ok(());
    }
    let evidence: Value = recovery::read_json(&path)?;
    if context.spec.constraints.environment_change.is_none()
        || evidence["source_commit"] != git(&context.workspace, &["rev-parse", "HEAD"])?
        || evidence["proposed_lock_digest"]
            != ayni_orchestrator_domain::digest::sha256(&read_bounded(
                &context.workspace.join(".ayni.lock"),
                managed_runtime::MAX_METADATA_BYTES,
            )?)
        || evidence["baseline_runtime_digest"] != context.spec.runtime_digest
    {
        return Err(io::Error::other(
            "retained environment-change evidence is corrupt",
        ));
    }
    Ok(())
}

fn dependency_inputs_digest(context: &Context) -> io::Result<String> {
    let mut inputs = Vec::new();
    if let Some(scope) = &context.spec.constraints.environment_change {
        for path in &scope.dependency_paths {
            let digest = match read_bounded(
                &context.workspace.join(path),
                managed_runtime::MAX_METADATA_BYTES,
            ) {
                Ok(bytes) => Some(ayni_orchestrator_domain::digest::sha256(&bytes)),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            };
            inputs.push((path, digest));
        }
    }
    Ok(ayni_orchestrator_domain::digest::sha256(
        &serde_json::to_vec(&inputs)?,
    ))
}

fn validate_proposal(context: &Context) -> io::Result<()> {
    if context.spec.constraints.environment_change.is_none() {
        return Ok(());
    }
    validate_policy(context)?;
    let baseline = git_bytes(
        &context.workspace,
        &[
            "show",
            &format!("{}:.ayni.lock", context.spec.execution.revision),
        ],
    )?;
    let proposed = read_bounded(
        &context.workspace.join(".ayni.lock"),
        managed_runtime::MAX_METADATA_BYTES,
    )?;
    let path = context.output.join("evidence/environment-change.json");
    if !path.exists() {
        if proposed != baseline {
            return Err(io::Error::other(
                "protected proposed lock lacks worker regeneration evidence",
            ));
        }
        return Ok(());
    }
    let evidence: Value = recovery::read_json(&path)?;
    if evidence["proposed_lock_digest"] != ayni_orchestrator_domain::digest::sha256(&proposed)
        || evidence["dependency_inputs_digest"] != dependency_inputs_digest(context)?
        || (proposed != baseline && evidence["lock_regeneration"] != "succeeded")
    {
        return Err(io::Error::other(
            "protected proposed lock regeneration evidence does not match final inputs",
        ));
    }
    Ok(())
}

fn baseline_base(lock: &Value) -> io::Result<String> {
    let field = |name| {
        lock["provisioning_base"][name]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| io::Error::other("baseline provisioning base missing"))
    };
    Ok(format!("{}@{}", field("reference")?, field("digest")?))
}

#[cfg(test)]
mod tests;
