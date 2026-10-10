//! Durable public signing trust proposals. Private material never enters this state.
use super::{
    Digest, ExecutionBackend, ExecutionRequest, ProfileRuntime, RuntimeCursor, RuntimeExpectation,
    RuntimeReconciler, Sha256, operation_name, v1,
};
use crate::profiles::ayni::preparation_state::{SigningIdentity, TrustResult, TrustUpdate};
use ayni_orchestrator_application::execution::ExecutionState;

pub(super) fn ready_certificate(
    certificate: Option<&SigningIdentity>,
) -> Result<&SigningIdentity, &'static str> {
    let certificate = certificate.ok_or("certificate_setup_required")?;
    if !certificate.enabled || certificate.status != "ready" {
        return Err("certificate_not_ready");
    }
    Ok(certificate)
}
#[derive(Clone, Copy)]
struct TrustContext<'a> {
    id: &'a str,
    runtime: &'a ProfileRuntime,
    github: &'a v1::GithubRepository,
    branch: &'a str,
    head: &'a str,
    token: &'a str,
    certificate: &'a SigningIdentity,
    policy: &'a [u8],
    lock: &'a [u8],
}

impl<E: ExecutionBackend> RuntimeReconciler<E> {
    #[allow(
        clippy::too_many_arguments,
        clippy::too_many_lines,
        reason = "one trust transition binds provider, source and certificate identities before external work"
    )]
    pub(super) async fn ensure_certificate_trust(
        &self,
        id: &str,
        runtime: &ProfileRuntime,
        github: &v1::GithubRepository,
        branch: &str,
        head: &str,
        token: &str,
        certificate: &SigningIdentity,
        cursor: &mut RuntimeCursor,
    ) -> Result<bool, &'static str> {
        let default_head = head;
        let merged_head = cursor
            .trust
            .as_ref()
            .and_then(|trust| trust.merge_revision.clone());
        let head = merged_head.as_deref().unwrap_or(head);
        let policy = self
            .source
            .file(&github.owner, &github.name, head, ".ayni.toml", token)
            .await
            .map_err(|reason| {
                if reason == "github_not_found" {
                    "policy_missing"
                } else {
                    reason
                }
            })?;
        let pinned = policy_matches(&policy, certificate)?;
        let lock = self
            .source
            .lock(&github.owner, &github.name, head, token)
            .await
            .map_err(|reason| {
                if reason == "github_not_found" {
                    "lock_missing"
                } else {
                    reason
                }
            })?;
        crate::profiles::ayni::environment::parse_lock(&lock)
            .map_err(|_| "repository_lock_invalid")?;
        let generation = format!(
            "{:x}",
            Sha256::digest(format!(
                "{}\0{}\0{}\0{}\0{}\0{}\0{}",
                github.repository_id,
                certificate.id,
                certificate.revision,
                certificate.key_id,
                certificate.public_key,
                runtime.platform,
                runtime
                    .build
                    .as_ref()
                    .ok_or("runtime_builder_missing")?
                    .image
            ))
        );
        if cursor
            .trust
            .as_ref()
            .is_none_or(|trust| trust.generation != generation)
        {
            if let Some(old) = &cursor.trust {
                self.kubernetes.cleanup(&old.job).await?;
            }
            cursor.trust = Some(TrustUpdate {
                generation: generation.clone(),
                head: head.into(),
                job: operation_name(&format!("trust:{generation}:{head}"), 0),
                attempt: 0,
                result: None,
                pull_number: None,
                state: "pending".into(),
                merge_revision: None,
            });
            cursor.status.remove("certificate-trust-pr");
            self.save(id, cursor).await?;
        }
        let mut trust = cursor
            .trust
            .clone()
            .ok_or("certificate_trust_state_invalid")?;
        let context = TrustContext {
            id,
            runtime,
            github,
            branch,
            head,
            token,
            certificate,
            policy: &policy,
            lock: &lock,
        };
        // A previously generated/merged policy must still pin exactly the selected key.
        if trust.state == "verified" && pinned && trust.head == head {
            let retired = cursor.status.remove("certificate-trust-pr").is_some();
            if retired
                || cursor
                    .status
                    .get("certificate-trust-state")
                    .map(String::as_str)
                    != Some("verified")
            {
                cursor
                    .status
                    .insert("certificate-trust-state".into(), "verified".into());
                self.save(id, cursor).await?;
            }
            self.kubernetes.cleanup(&trust.job).await?;
            if default_head != head {
                trust.merge_revision = None;
                trust.reset(default_head);
                cursor
                    .status
                    .insert("certificate-trust-state".into(), trust.state.clone());
                cursor.trust = Some(trust);
                self.save(id, cursor).await?;
                return Ok(false);
            }
            return Ok(true);
        }
        if trust.state == "verified" || (trust.head != head && trust.pull_number.is_none()) {
            self.kubernetes.cleanup(&trust.job).await?;
            trust.reset(head);
            cursor.status.remove("certificate-trust-pr");
        }
        self.observe_trust_pull(&context, &mut trust, cursor)
            .await?;
        if trust
            .merge_revision
            .as_deref()
            .is_some_and(|revision| revision != head)
        {
            cursor.trust = Some(trust);
            self.save(id, cursor).await?;
            return Ok(false);
        }
        if trust.state == "open" && trust.head != head {
            self.kubernetes.cleanup(&trust.job).await?;
            trust.reset(head);
            cursor.status.remove("certificate-trust-pr");
        }
        self.retry_trust_update(&context, &mut trust, cursor)
            .await?;
        self.observe_trust_job(&context, &mut trust, cursor).await?;
        // Freeze the reviewed output before asking GitHub for write authority.
        cursor.trust = Some(trust.clone());
        cursor
            .status
            .insert("certificate-trust-state".into(), trust.state.clone());
        self.save(id, cursor).await?;
        self.propose_trust_pull(&context, &mut trust, cursor)
            .await?;
        if trust.state == "failed" {
            return Err("certificate_trust_lock_failed");
        }
        if trust.state == "closed" {
            return Err("certificate_trust_pull_closed");
        }
        Ok(trust.state == "verified" && default_head == head)
    }
    async fn retry_trust_update(
        &self,
        context: &TrustContext<'_>,
        trust: &mut TrustUpdate,
        cursor: &mut RuntimeCursor,
    ) -> Result<(), &'static str> {
        let TrustContext {
            id: _,
            runtime: _,
            github,
            branch: _,
            head,
            token: _,
            certificate: _,
            policy: _,
            lock: _,
        } = *context;
        if trust.state == "closed" {
            if !cursor.retry_trust {
                return Err("certificate_trust_pull_closed");
            }
            let write_token = self.source.trust_token(github).await?;
            self.source
                .reopen_trust_pull(
                    github,
                    trust.pull_number.ok_or("certificate_trust_pull_invalid")?,
                    &write_token,
                )
                .await?;
            trust.state = "open".into();
            cursor.retry_trust = false;
        }
        if trust.state == "failed" && cursor.retry_trust {
            self.kubernetes.cleanup(&trust.job).await?;
            trust.reset(head);
            cursor.retry_trust = false;
        }
        Ok(())
    }

    async fn observe_trust_pull(
        &self,
        context: &TrustContext<'_>,
        trust: &mut TrustUpdate,
        cursor: &mut RuntimeCursor,
    ) -> Result<(), &'static str> {
        let TrustContext {
            id: _,
            runtime: _,
            github,
            branch: _,
            head,
            token: _,
            certificate,
            policy,
            lock: _,
        } = *context;
        if let Some(number) = trust.pull_number {
            let pull_token = self.source.trust_read_token(github).await?;
            let pull = self
                .source
                .pull(&github.owner, &github.name, number, &pull_token)
                .await?;
            let merged = pull["merged_at"].as_str().is_some();
            let merged_revision = if merged {
                let revision = pull["merge_commit_sha"]
                    .as_str()
                    .ok_or("certificate_trust_merge_invalid")?;
                ayni_orchestrator_application::identity::GitRevision::parse(revision)
                    .map_err(|_| "certificate_trust_merge_invalid")?;
                let merged_policy = self
                    .source
                    .file(
                        &github.owner,
                        &github.name,
                        revision,
                        ".ayni.toml",
                        &pull_token,
                    )
                    .await?;
                if !policy_matches(&merged_policy, certificate)? {
                    return Err("certificate_trust_merge_mismatch");
                }
                Some(revision.to_owned())
            } else {
                None
            };
            trust.observe_pull(
                merged,
                pull["state"] == "open",
                merged || policy_matches(policy, certificate)?,
                merged_revision.as_deref().unwrap_or(head),
            )?;
            if merged {
                trust.merge_revision = merged_revision;
                cursor.status.remove("certificate-trust-pr");
            }
            if merged {
                cursor
                    .status
                    .insert("certificate-trust-state".into(), "merged".into());
            }
        }
        Ok(())
    }

    async fn observe_trust_job(
        &self,
        context: &TrustContext<'_>,
        trust: &mut TrustUpdate,
        cursor: &mut RuntimeCursor,
    ) -> Result<(), &'static str> {
        let TrustContext {
            id,
            runtime,
            github,
            branch: _,
            head,
            token,
            certificate,
            policy,
            lock,
        } = *context;
        if trust.state == "pending" {
            match self.kubernetes.coding_state(&trust.job).await? {
                ExecutionState::Missing => {
                    let request = ExecutionRequest {
                        repository_url: format!(
                            "https://github.com/{}/{}.git",
                            github.owner, github.name
                        ),
                        revision: trust.head.clone(),
                        expected: RuntimeExpectation {
                            lock_fingerprint: format!(
                                "sha256:{}",
                                crate::profiles::ayni::environment::parse_lock(lock)
                                    .map_err(|_| "repository_lock_invalid")?
                                    .fingerprint
                            ),
                            platform: runtime.platform.clone(),
                            ayni_version: crate::profiles::ayni::environment::parse_lock(lock)
                                .map_err(|_| "repository_lock_invalid")?
                                .ayni_version,
                        },
                    };
                    self.kubernetes
                        .start_trust_lock(
                            &trust.job,
                            &runtime.execution_runtime(),
                            &request,
                            token,
                            &certificate.key_id,
                            &certificate.public_key,
                        )
                        .await?;
                }
                ExecutionState::Running => {}
                ExecutionState::Failed => {
                    trust.state = "failed".into();
                }
                ExecutionState::Succeeded => {
                    let result: TrustResult =
                        serde_json::from_str(&self.kubernetes.trust_lock_result(&trust.job).await?)
                            .map_err(|_| "certificate_trust_result_invalid")?;
                    validate_result(&result, &trust.head, certificate, policy, lock)?;
                    if head != trust.head {
                        self.kubernetes.cleanup(&trust.job).await?;
                        cursor.trust = None;
                        self.save(id, cursor).await?;
                        return Err("runtime_source_changed");
                    }
                    if result.policy.as_bytes() == policy && result.lock.as_bytes() == lock {
                        trust.state = "verified".into();
                        trust.result = None;
                        cursor.status.remove("certificate-trust-pr");
                        // Preserve the verified no-op before asynchronous deletion;
                        // the next pass must finish cleanup, not recreate the Job.
                        cursor.trust = Some(trust.clone());
                        cursor
                            .status
                            .insert("certificate-trust-state".into(), trust.state.clone());
                        self.save(id, cursor).await?;
                        self.kubernetes.cleanup(&trust.job).await?;
                    } else {
                        trust.result = Some(result);
                        trust.state = "propose".into();
                    }
                }
            }
        }
        Ok(())
    }

    async fn propose_trust_pull(
        &self,
        context: &TrustContext<'_>,
        trust: &mut TrustUpdate,
        cursor: &mut RuntimeCursor,
    ) -> Result<(), &'static str> {
        let TrustContext {
            id,
            runtime: _,
            github,
            branch,
            head: _,
            token: _,
            certificate: _,
            policy: _,
            lock: _,
        } = *context;
        if trust.state == "propose" {
            let result = trust
                .result
                .as_ref()
                .ok_or("certificate_trust_result_invalid")?;
            let write_token = self.source.trust_token(github).await?;
            let (owned_branch, existing) = self
                .source
                .find_trust_pull(github, branch, &TrustUpdate::branch(), &write_token)
                .await?;
            self.source
                .trust_branch(
                    github,
                    &owned_branch,
                    &trust.head,
                    &result.policy,
                    &result.lock,
                    &write_token,
                )
                .await?;
            let pull = if let Some(pull) = existing {
                pull
            } else {
                self.source.create_pull(&github.owner,&github.name,&write_token,&serde_json::json!({"head":owned_branch,"base":branch,"title":"feat(ayni): update environment signing trust","body":"Pin the selected environment signing public key and refresh the environment lock with the supported Ayni CLI. Review and merge to activate the signed runtime.","draft":true})).await?
            };
            let number = pull["number"]
                .as_u64()
                .ok_or("certificate_trust_pull_invalid")?;
            trust.pull_number = Some(number);
            trust.state = if pull["state"] == "open" {
                "open"
            } else {
                "closed"
            }
            .into();
            cursor.status.insert(
                "certificate-trust-pr".into(),
                format!(
                    "https://github.com/{}/{}/pull/{number}",
                    github.owner, github.name
                ),
            );
            self.kubernetes.cleanup(&trust.job).await?;
            // Provider branch now retains reviewed files; only safe status remains in SQLite.
            trust.result = None;
            cursor.trust = Some(trust.clone());
            cursor
                .status
                .insert("certificate-trust-state".into(), trust.state.clone());
            self.save(id, cursor).await?;
        }
        Ok(())
    }
}

fn policy_matches(bytes: &[u8], certificate: &SigningIdentity) -> Result<bool, &'static str> {
    let policy: toml_edit::DocumentMut = std::str::from_utf8(bytes)
        .map_err(|_| "repository_policy_invalid")?
        .parse()
        .map_err(|_| "repository_policy_invalid")?;
    Ok(policy
        .get("environment")
        .and_then(|v| v.get("certificate"))
        .and_then(|v| v.get("trusted_keys"))
        .and_then(|v| v.get(&certificate.key_id))
        .and_then(toml_edit::Item::as_str)
        == Some(certificate.public_key.as_str()))
}
fn validate_result(
    result: &TrustResult,
    head: &str,
    certificate: &SigningIdentity,
    original_policy: &[u8],
    original_lock: &[u8],
) -> Result<(), &'static str> {
    if result.revision != head
        || result.policy.len() > 1_048_576
        || result.lock.len() > 1_048_576
        || !policy_matches(result.policy.as_bytes(), certificate)?
    {
        return Err("certificate_trust_result_invalid");
    }
    let mut expected: toml_edit::DocumentMut = std::str::from_utf8(original_policy)
        .map_err(|_| "repository_policy_invalid")?
        .parse()
        .map_err(|_| "repository_policy_invalid")?;
    let mut ancestor = expected.as_item();
    for field in ["environment", "certificate", "trusted_keys"] {
        let Some(item) = ancestor.get(field) else {
            break;
        };
        if !item.is_table_like() {
            return Err("repository_policy_invalid");
        }
        ancestor = item;
    }
    expected["environment"]["certificate"]["trusted_keys"][&certificate.key_id] =
        toml_edit::value(&certificate.public_key);
    if expected.to_string() != result.policy {
        return Err("certificate_trust_policy_changed");
    }
    let old: serde_json::Value =
        serde_json::from_slice(original_lock).map_err(|_| "repository_lock_invalid")?;
    let new: serde_json::Value =
        serde_json::from_str(&result.lock).map_err(|_| "certificate_trust_result_invalid")?;
    for field in [
        "provisioning_base",
        "targets",
        "mise_version",
        "recipe_version",
        "platforms",
        "ayni_version",
    ] {
        if old[field] != new[field] {
            return Err("certificate_trust_immutable_inputs_changed");
        }
    }
    crate::profiles::ayni::environment::parse_lock(result.lock.as_bytes())
        .map_err(|_| "certificate_trust_result_invalid")?;
    Ok(())
}

impl<E: ExecutionBackend> RuntimeReconciler<E> {
    #[allow(
        clippy::too_many_arguments,
        reason = "one preparation phase binds frozen repository and certificate inputs"
    )]
    pub(super) async fn prepare_certificate_trust(
        &self,
        id: &str,
        runtime: &mut ProfileRuntime,
        github: &v1::GithubRepository,
        branch: &str,
        head: &str,
        token: &str,
        certificate: Option<&SigningIdentity>,
        cursor: &mut RuntimeCursor,
    ) -> Result<bool, &'static str> {
        if cursor.pending.is_none() {
            let certificate = ready_certificate(certificate)?;
            if let Some(build) = &mut runtime.build {
                build.signing_secret.clone_from(&certificate.secret_name);
            }
            if !self
                .ensure_certificate_trust(
                    id,
                    runtime,
                    github,
                    branch,
                    head,
                    token,
                    certificate,
                    cursor,
                )
                .await?
            {
                let lookup_id = id.to_owned();
                self.with_database(move |store| {
                    store
                        .runtime_failure(&lookup_id, "trust_policy_missing")
                        .map_err(|_| "runtime_cursor_invalid")
                })
                .await?;
                cursor
                    .status
                    .insert("reason-code".into(), "trust_policy_missing".into());
                cursor
                    .status
                    .insert("phase".into(), "setup_required".into());
                self.save(id, cursor).await?;
                return Ok(false);
            }
            cursor.status.remove("reason-code");
            cursor.status.insert("phase".into(), "analyzing".into());
            self.save(id, cursor).await?;
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::{SigningIdentity, TrustResult, policy_matches, validate_result};
    fn certificate() -> SigningIdentity {
        SigningIdentity {
            id: "certificate-one".into(),
            revision: "1".into(),
            key_id: "release".into(),
            public_key: "a".repeat(64),
            secret_name: "signing-one-1".into(),
            enabled: true,
            status: "ready".into(),
        }
    }
    fn lock() -> String {
        serde_json::json!({"schema_version":"0.9.0","recipe_version":"2","fingerprint":format!("sha256:{}","b".repeat(64)),"provisioning_base":{"reference":"registry.test/base","digest":format!("sha256:{}","c".repeat(64))},"targets":[],"mise_version":"2026.6.14","platforms":[],"ayni_version":"0.15.0"}).to_string()
    }
    fn result(policy: &str) -> Result<TrustResult, toml_edit::TomlError> {
        let mut document: toml_edit::DocumentMut = policy.parse()?;
        document["environment"]["certificate"]["trusted_keys"]["release"] =
            toml_edit::value("a".repeat(64));
        Ok(TrustResult {
            revision: "b".repeat(40),
            policy: document.to_string(),
            lock: lock(),
        })
    }
    #[test]
    fn proposal_preserves_unrelated_policy_and_immutable_lock_inputs()
    -> Result<(), Box<dyn std::error::Error>> {
        let original = "# preserve this comment\n[environment.certificate.trusted_keys]\nother = 'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd'\n";
        let mut proposed = result(original)?;
        let certificate = certificate();
        assert!(
            validate_result(
                &proposed,
                &"b".repeat(40),
                &certificate,
                original.as_bytes(),
                lock().as_bytes()
            )
            .is_ok()
        );
        proposed.policy = proposed.policy.replace("other =", "different =");
        assert_eq!(
            validate_result(
                &proposed,
                &"b".repeat(40),
                &certificate,
                original.as_bytes(),
                lock().as_bytes()
            ),
            Err("certificate_trust_policy_changed")
        );
        let mut proposed = result(original)?;
        proposed.lock = proposed
            .lock
            .replace("registry.test/base", "registry.test/changed");
        assert_eq!(
            validate_result(
                &proposed,
                &"b".repeat(40),
                &certificate,
                original.as_bytes(),
                lock().as_bytes()
            ),
            Err("certificate_trust_immutable_inputs_changed")
        );
        Ok(())
    }
    #[test]
    fn scalar_policy_ancestors_fail_closed_without_panicking()
    -> Result<(), Box<dyn std::error::Error>> {
        let proposed = result("[environment]\n")?;
        assert_eq!(
            validate_result(
                &proposed,
                &"b".repeat(40),
                &certificate(),
                b"[environment]\ncertificate = 'invalid'\n",
                lock().as_bytes()
            ),
            Err("repository_policy_invalid")
        );
        Ok(())
    }
    #[test]
    fn malformed_policy_and_changed_public_key_fail_closed() {
        let certificate = certificate();
        assert_eq!(
            policy_matches(b"[invalid", &certificate),
            Err("repository_policy_invalid")
        );
        assert_eq!(policy_matches(b"[environment]\n", &certificate), Ok(false));
        let policy = format!(
            "[environment.certificate.trusted_keys]\nrelease = '{}'\n",
            certificate.public_key
        );
        assert_eq!(policy_matches(policy.as_bytes(), &certificate), Ok(true));
        let mut rotated = certificate;
        rotated.public_key = "e".repeat(64);
        assert_eq!(policy_matches(policy.as_bytes(), &rotated), Ok(false));
    }
    fn trust() -> super::TrustUpdate {
        super::TrustUpdate {
            generation: "a".repeat(64),
            head: "b".repeat(40),
            job: "trust-one".into(),
            attempt: 0,
            result: None,
            pull_number: Some(17),
            state: "open".into(),
            merge_revision: None,
        }
    }
    #[test]
    fn merged_pull_revalidates_current_policy_before_retiring_provider_identity()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut trust = trust();
        assert_eq!(
            trust.observe_pull(true, false, false, &"c".repeat(40)),
            Err("certificate_trust_merge_mismatch")
        );
        assert_eq!(trust.pull_number, Some(17));
        trust.observe_pull(true, false, true, &"c".repeat(40))?;
        assert_eq!(trust.state, "pending");
        assert_eq!(trust.pull_number, None);
        assert_eq!(trust.head, "c".repeat(40));
        assert_eq!(trust.attempt, 1);
        Ok(())
    }
    #[test]
    fn externally_merged_closed_pull_revalidates_without_operator_retry()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut trust = trust();
        trust.observe_pull(false, false, false, &"b".repeat(40))?;
        assert_eq!(trust.state, "closed");
        trust.observe_pull(true, false, true, &"c".repeat(40))?;
        assert_eq!(trust.state, "pending");
        assert_eq!(trust.pull_number, None);
        Ok(())
    }
    #[test]
    fn restart_preserves_dedup_identity_and_closed_pull_observation()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut trust = trust();
        trust.observe_pull(false, false, false, &"b".repeat(40))?;
        assert_eq!(trust.state, "closed");
        let saved = serde_json::to_string(&trust)?;
        let mut restarted: super::TrustUpdate = serde_json::from_str(&saved)?;
        restarted.observe_pull(false, true, false, &"b".repeat(40))?;
        assert_eq!(restarted.state, "open");
        assert_eq!(restarted.pull_number, Some(17));
        restarted.reset(&"b".repeat(40));
        assert_eq!(restarted.state, "pending");
        assert_ne!(restarted.job, trust.job);
        assert!(!saved.contains("signing_seed"));
        Ok(())
    }
}
