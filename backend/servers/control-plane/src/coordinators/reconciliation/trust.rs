//! Durable public signing trust proposals. Private material never enters this state.
use super::{
    Deserialize, Digest, ExecutionBackend, ExecutionRequest, ProfileRuntime, RuntimeCursor,
    RuntimeExpectation, RuntimeReconciler, Serialize, Sha256, operation_name, v1,
};
use ayni_orchestrator_application::execution::ExecutionState;

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct SigningIdentity {
    pub id: String,
    pub revision: String,
    pub key_id: String,
    pub public_key: String,
    pub secret_name: String,
    pub enabled: bool,
    pub status: String,
}
impl From<v1::Certificate> for SigningIdentity {
    fn from(certificate: v1::Certificate) -> Self {
        Self {
            id: certificate.id,
            revision: certificate.revision,
            key_id: certificate.key_id,
            public_key: certificate.public_key,
            secret_name: certificate.secret_name,
            enabled: certificate.enabled,
            status: certificate.status,
        }
    }
}
#[derive(Clone, Deserialize, Serialize)]
pub(super) struct TrustUpdate {
    generation: String,
    head: String,
    job: String,
    #[serde(default)]
    attempt: u64,
    result: Option<TrustResult>,
    pull_number: Option<u64>,
    state: String,
}
impl TrustUpdate {
    fn reset(&mut self, head: &str) {
        self.attempt = self.attempt.saturating_add(1);
        self.head = head.into();
        self.job = operation_name(&format!("trust:{}:{head}", self.generation), self.attempt);
        self.result = None;
        self.pull_number = None;
        self.state = "pending".into();
    }

    fn branch(&self) -> String {
        format!("setup/certificate-{}", &self.generation[..24])
    }

    fn after_merged_branch(&self, number: u64) -> String {
        format!("{}-after-{number}", self.branch())
    }

    fn observe_pull(
        &mut self,
        merged: bool,
        open: bool,
        pinned: bool,
        head: &str,
    ) -> Result<(), &'static str> {
        if merged {
            if !pinned {
                return Err("certificate_trust_merge_mismatch");
            }
            self.reset(head);
        } else {
            self.state = if open { "open" } else { "closed" }.into();
        }
        Ok(())
    }
}

#[derive(Clone, Deserialize, Serialize)]
struct TrustResult {
    revision: String,
    policy: String,
    lock: String,
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
    #[allow(clippy::too_many_arguments)]
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
        let policy = self
            .source
            .file(&github.owner, &github.name, head, ".ayni.toml", token)
            .await
            .map_err(|reason| {
                if reason == "github_not_found" {
                    "certificate_repository_not_ayni"
                } else {
                    reason
                }
            })?;
        let pinned = policy_matches(&policy, certificate)?;
        let lock = self
            .source
            .lock(&github.owner, &github.name, head, token)
            .await?;
        crate::profiles::ayni::environment::parse_lock(&lock)
            .map_err(|_| "repository_lock_invalid")?;
        let generation = format!(
            "{:x}",
            Sha256::digest(format!(
                "{}\0{}\0{}\0{}\0{}",
                github.repository_id,
                certificate.id,
                certificate.revision,
                certificate.key_id,
                certificate.public_key
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
            return Ok(true);
        }
        if trust.state == "verified" || (trust.head != head && trust.pull_number.is_none()) {
            self.kubernetes.cleanup(&trust.job).await?;
            trust.reset(head);
            cursor.status.remove("certificate-trust-pr");
        }
        self.observe_trust_pull(&context, &mut trust, cursor)
            .await?;
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
        Ok(trust.state == "verified")
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
            if !cursor.force_rebuild {
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
            cursor.force_rebuild = false;
        }
        if trust.state == "failed" && cursor.force_rebuild {
            self.kubernetes.cleanup(&trust.job).await?;
            trust.reset(head);
            cursor.force_rebuild = false;
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
            trust.observe_pull(
                merged,
                pull["state"] == "open",
                policy_matches(policy, certificate)?,
                head,
            )?;
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
                            lock_fingerprint: String::new(),
                            platform: runtime.platform.clone(),
                            ayni_version: ayni_orchestrator_profile_ayni::ayni_release::CLI_VERSION
                                .into(),
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

    async fn available_trust_pull(
        &self,
        github: &v1::GithubRepository,
        base: &str,
        trust: &TrustUpdate,
        token: &str,
    ) -> Result<(String, Option<serde_json::Value>), &'static str> {
        let mut branch = trust.branch();
        // Provider identity, rather than a runtime's local job retry count, deduplicates
        // proposals shared by multiple profiles and survives policy reversion/restart.
        for _ in 0..32 {
            let pull = self
                .source
                .find_pull(&github.owner, &github.name, &branch, base, token)
                .await?;
            if let Some(ref merged) = pull
                && merged["merged_at"].as_str().is_some()
            {
                branch = trust.after_merged_branch(
                    merged["number"]
                        .as_u64()
                        .ok_or("certificate_trust_pull_invalid")?,
                );
                continue;
            }
            return Ok((branch, pull));
        }
        Err("certificate_trust_history_limit")
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
                .available_trust_pull(github, branch, trust, &write_token)
                .await?;
            let pull = if let Some(pull) = existing {
                pull
            } else {
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
        serde_json::json!({"schema_version":"0.9.0","recipe_version":"2","fingerprint":format!("sha256:{}","b".repeat(64)),"provisioning_base":{"reference":"registry.test/base","digest":format!("sha256:{}","c".repeat(64))},"targets":[],"mise_version":"2026.6.14","platforms":[],"ayni_version":"0.14.5"}).to_string()
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
    fn profiles_at_different_heads_share_the_desired_generation_proposal() {
        let first = trust();
        let mut later = first.clone();
        later.reset(&"c".repeat(40));
        assert_ne!(later.head, first.head);
        assert_ne!(later.attempt, first.attempt);
        assert_eq!(later.branch(), first.branch());
        assert_eq!(later.after_merged_branch(17), first.after_merged_branch(17));
    }
    #[test]
    fn merged_provider_identity_distinguishes_reversion_without_job_retry_duplicates() {
        let mut trust = trust();
        let old_branch = trust.branch();
        trust.state = "verified".into();
        trust.reset(&"b".repeat(40));
        assert_eq!(trust.state, "pending");
        assert_eq!(trust.pull_number, None);
        assert_eq!(trust.branch(), old_branch);
        assert_ne!(trust.after_merged_branch(17), old_branch);
        assert_ne!(trust.after_merged_branch(17), trust.after_merged_branch(18));
        assert_eq!(trust.attempt, 1);
    }
    #[test]
    fn restart_preserves_dedup_identity_and_closed_pull_observation()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut trust = trust();
        trust.observe_pull(false, false, false, &"b".repeat(40))?;
        assert_eq!(trust.state, "closed");
        let saved = serde_json::to_string(&trust)?;
        let mut restarted: super::TrustUpdate = serde_json::from_str(&saved)?;
        assert_eq!(restarted.branch(), trust.branch());
        restarted.observe_pull(false, true, false, &"b".repeat(40))?;
        assert_eq!(restarted.state, "open");
        assert_eq!(restarted.pull_number, Some(17));
        assert_eq!(restarted.branch(), trust.branch());
        restarted.reset(&"b".repeat(40));
        assert_eq!(restarted.state, "pending");
        assert_ne!(restarted.job, trust.job);
        assert!(!saved.contains("signing_seed"));
        Ok(())
    }
}
