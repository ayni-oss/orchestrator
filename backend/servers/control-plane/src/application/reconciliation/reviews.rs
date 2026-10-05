use super::{BTreeMap, Digest, ExecutionBackend, RuntimeReconciler, Sha256, v1};
use ayni_orchestrator_actions_contract as assessment;
use ayni_orchestrator_actions_contract::{Admission, Manifest};

impl<E: ExecutionBackend> RuntimeReconciler<E> {
    pub(super) async fn review_assessments(&self) -> Result<(), &'static str> {
        for (profile, runtime) in &self.configured_profiles()? {
            let Some(policy) = &runtime.assessment else {
                continue;
            };
            if let Err(reason) = self.review_profile(profile, policy).await {
                tracing::warn!(reason_code = reason, "assessment reconciliation deferred");
            }
        }
        Ok(())
    }

    pub(super) async fn review_profile(
        &self,
        profile_id: &str,
        policy: &crate::process::runtime_configuration::AssessmentConfiguration,
    ) -> Result<(), &'static str> {
        let (repository, automation) = {
            let mut store = self
                .state
                .store
                .lock()
                .map_err(|_| "review_store_unavailable")?;
            let profile = store
                .runtime_profile(profile_id)
                .map_err(|_| "review_profile_missing")?;
            let repository = store
                .repository_connection(&profile.repository_connection_id)
                .map_err(|_| "review_repository_missing")?;
            let installations = store
                .list_automation_installations(profile_id, None)
                .map_err(|_| "review_automation_missing")?;
            drop(store);
            let automation = installations
                .items
                .into_iter()
                .find(|a| {
                    a.version == crate::profiles::ayni::onboarding_templates::TEMPLATE_VERSION
                        && a.state == v1::AutomationInstallationState::Installed as i32
                })
                .ok_or("review_automation_unverified")?;
            (repository, automation)
        };
        let Some(v1::repository_provider::Provider::Github(github)) = repository
            .provider
            .as_ref()
            .and_then(|p| p.provider.as_ref())
        else {
            return Err("review_provider_unsupported");
        };
        if !matches!(automation.provider.as_ref(), Some(v1::automation_installation::Provider::Github(a)) if a.installation_id == github.installation_id && a.assessment_workflow_path == crate::profiles::ayni::onboarding_templates::ASSESSMENT_WORKFLOW_PATH)
        {
            return Err("review_automation_mismatch");
        }
        let token = self
            .source
            .review_token(github.installation_id, &github.repository_id)
            .await?;
        let runs = self.source.review_get(github, "actions/workflows/ayni-managed-assessment.yml/runs?event=workflow_run&status=completed&per_page=100", &token).await?;
        let runs = runs["workflow_runs"]
            .as_array()
            .ok_or("review_runs_invalid")?;
        for run in runs.iter().rev() {
            if let Err(reason) = self
                .import_run(profile_id, policy, &repository, github, run, &token)
                .await
            {
                tracing::warn!(reason_code = reason, "assessment run rejected");
            }
        }
        Ok(())
    }

    pub(super) async fn download_review_evidence(
        &self,
        github: &v1::GithubRepository,
        run_id: u64,
        attempt: u64,
        token: &str,
    ) -> Result<EvidenceBundle, &'static str> {
        let items = self
            .source
            .review_get(
                github,
                &format!("actions/runs/{run_id}/artifacts?per_page=10"),
                token,
            )
            .await?;
        let items = items["artifacts"]
            .as_array()
            .filter(|items| items.len() < 10)
            .ok_or("review_artifact_limit")?;
        let name = format!("assessment-{run_id}-{attempt}");
        let selected: Vec<_> = items
            .iter()
            .filter(|a| a["name"] == name && a["expired"] == false)
            .collect();
        if selected.len() != 1 {
            return Err("review_artifact_missing");
        }
        let item = selected[0];
        if item["size_in_bytes"]
            .as_u64()
            .is_none_or(|s| s > 12 * 1024 * 1024)
            || item["workflow_run"]["id"] != run_id
        {
            return Err("review_artifact_unbound");
        }
        let archive = self
            .source
            .review_archive(
                github,
                item["id"].as_u64().ok_or("review_artifact_invalid")?,
                token,
            )
            .await?;
        if item["digest"].as_str() != Some(&assessment::sha256(&archive)) {
            return Err("review_archive_digest_mismatch");
        }
        unpack_assessment(&archive)
    }

    pub(super) async fn verify_review_workflow(
        &self,
        repository: &v1::RepositoryConnection,
        github: &v1::GithubRepository,
        run: &serde_json::Value,
        token: &str,
    ) -> Result<String, &'static str> {
        let workflow_sha = run["head_sha"]
            .as_str()
            .filter(|s| assessment::hex(s, 40))
            .ok_or("review_run_invalid")?;
        if run["event"] != "workflow_run"
            || run["path"] != crate::profiles::ayni::onboarding_templates::ASSESSMENT_WORKFLOW_PATH
            || run["repository"]["id"]
                .as_u64()
                .map(|id| id.to_string())
                .as_deref()
                != Some(&github.repository_id)
        {
            return Err("review_workflow_untrusted");
        }
        // A candidate-supplied path, workflow name or manifest declaration is
        // insufficient. Compare all trusted program bytes at the provider's SHA.
        let templates = crate::profiles::ayni::onboarding_templates::render(
            crate::profiles::ayni::onboarding_templates::TEMPLATE_VERSION,
            &repository.default_branch,
        )
        .map_err(|_| "review_template_invalid")?;
        for (path, expected) in templates.files {
            let actual = self
                .source
                .file(&github.owner, &github.name, workflow_sha, path, token)
                .await?;
            if actual != expected.as_bytes() {
                return Err("review_workflow_untrusted");
            }
        }
        Ok(workflow_sha.to_owned())
    }

    pub(super) fn review_runtime(
        &self,
        profile_id: &str,
        declared: &Manifest,
    ) -> Result<(v1::RuntimeProfileRevision, v1::RuntimeArtifact), &'static str> {
        let result = {
            let mut store = self
                .state
                .store
                .lock()
                .map_err(|_| "review_store_unavailable")?;
            let revision = store
                .runtime_profile_revision(&declared.run.profile_revision)
                .map_err(|_| "review_revision_missing")?;
            if revision.runtime_profile_id != profile_id {
                return Err("review_profile_mismatch");
            }
            let artifacts = store
                .list_runtime_artifacts(
                    profile_id,
                    Some(&revision.id),
                    Some(v1::RuntimeArtifactStatus::Verified as i32),
                    None,
                )
                .map_err(|_| "review_runtime_missing")?;
            drop(store);
            let artifact = artifacts
                .items
                .into_iter()
                .find(|a| {
                    a.resolved_digest == declared.runtime_digest
                        && a.platform.as_ref().is_some_and(|p| {
                            p.operating_system == "linux" && p.architecture == "amd64"
                        })
                })
                .ok_or("review_runtime_unverified")?;
            (revision, artifact)
        };
        Ok(result)
    }

    pub(super) async fn verify_review_contract(
        &self,
        github: &v1::GithubRepository,
        token: &str,
        current_head: &str,
        declared: &Manifest,
        inputs: (&[u8], &[u8]),
    ) -> Result<(), &'static str> {
        if declared.run.head == current_head {
            let head_lock = self
                .source
                .lock(&github.owner, &github.name, current_head, token)
                .await?;
            let head_policy = self
                .source
                .file(
                    &github.owner,
                    &github.name,
                    current_head,
                    ".ayni.toml",
                    token,
                )
                .await?;
            if (head_lock != inputs.1 || head_policy != inputs.0)
                && declared.outcome != assessment::Outcome::Unsupported
            {
                return Err("review_candidate_policy_changed");
            }
        }
        Ok(())
    }

    pub(super) fn persist_review_import(
        &self,
        repository: &v1::RepositoryConnection,
        github: &v1::GithubRepository,
        pull: &serde_json::Value,
        bundle: &EvidenceBundle,
        manifest: &Manifest,
        summary: &assessment::Summary,
    ) -> Result<(v1::ChangeRequest, v1::Assessment), &'static str> {
        let current_head = pull["head"]["sha"].as_str().ok_or("review_head_invalid")?;
        let base = pull["base"]["sha"].as_str().ok_or("review_base_invalid")?;
        let (manifest_bytes, artifacts) = bundle;
        let state = provider_review_state(pull);
        let url = format!(
            "https://github.com/{}/{}/actions/runs/{}",
            github.owner, github.name, manifest.run.run_id
        );
        let (change, assessment) = {
            let mut store = self
                .state
                .store
                .lock()
                .map_err(|_| "review_store_unavailable")?;
            let change = store
                .observe_review(
                    &repository.id,
                    v1::GithubChangeRequest {
                        repository_id: github.repository_id.clone(),
                        pull_request_number: manifest.run.pull_request,
                        html_url: format!(
                            "https://github.com/{}/{}/pull/{}",
                            github.owner, github.name, manifest.run.pull_request
                        ),
                    },
                    current_head,
                    base,
                    state,
                )
                .map_err(|_| "review_store_failed")?;
            let assessment = store
                .import_review(&change, manifest, summary, manifest_bytes, artifacts, &url)
                .map_err(|_| "review_import_conflict")?;
            drop(store);
            (change, assessment)
        };
        Ok((change, assessment))
    }

    pub(super) async fn review_contract_inputs(
        &self,
        github: &v1::GithubRepository,
        token: &str,
        revision: &v1::RuntimeProfileRevision,
        artifact: &v1::RuntimeArtifact,
        artifacts: &BTreeMap<String, Vec<u8>>,
    ) -> Result<(Vec<u8>, Vec<u8>), &'static str> {
        let contract = self
            .source
            .file(
                &github.owner,
                &github.name,
                &revision.repository_revision,
                ".ayni.toml",
                token,
            )
            .await?;
        let lock = self
            .source
            .lock(
                &github.owner,
                &github.name,
                &revision.repository_revision,
                token,
            )
            .await?;
        let runtime_metadata = artifact
            .verified_metadata
            .iter()
            .find(|m| m.key == "runtime-metadata-digest")
            .ok_or("review_runtime_unverified")?;
        if let Some(metadata) = artifacts.get("runtime.json")
            && assessment::sha256(metadata) != runtime_metadata.value
        {
            return Err("review_runtime_mismatch");
        }
        Ok((contract, lock))
    }

    pub(super) async fn load_review_evidence(
        &self,
        github: &v1::GithubRepository,
        run_id: u64,
        attempt: u64,
        token: &str,
    ) -> Result<EvidenceBundle, &'static str> {
        let assessment_id = format!(
            "assessment:{:x}",
            Sha256::digest(format!("{}:{run_id}:{attempt}", github.repository_id))
        );
        let retained = self
            .state
            .store
            .lock()
            .map_err(|_| "review_store_unavailable")?
            .retained_review_evidence(&assessment_id)
            .map_err(|_| "review_retained_evidence_corrupt")?;
        let bundle = if let Some(retained) = retained {
            retained
        } else {
            self.download_review_evidence(github, run_id, attempt, token)
                .await?
        };
        Ok(bundle)
    }

    pub(super) async fn import_run(
        &self,
        profile_id: &str,
        policy: &crate::process::runtime_configuration::AssessmentConfiguration,
        repository: &v1::RepositoryConnection,
        github: &v1::GithubRepository,
        run: &serde_json::Value,
        token: &str,
    ) -> Result<(), &'static str> {
        let run_id = run["id"].as_u64().ok_or("review_run_invalid")?;
        let attempt = run["run_attempt"].as_u64().ok_or("review_run_invalid")?;
        let workflow_sha = self
            .verify_review_workflow(repository, github, run, token)
            .await?;
        let (manifest_bytes, artifacts) = self
            .load_review_evidence(github, run_id, attempt, token)
            .await?;
        let declared: Manifest =
            serde_json::from_slice(&manifest_bytes).map_err(|_| "review_manifest_invalid")?;
        let (revision, artifact) = self.review_runtime(profile_id, &declared)?;
        let Some(v1::runtime_profile_revision::Configuration::AyniManaged(configuration)) =
            revision.configuration.as_ref()
        else {
            return Err("review_profile_unsupported");
        };
        let (contract, lock) = self
            .review_contract_inputs(github, token, &revision, &artifact, &artifacts)
            .await?;
        let pull = self
            .source
            .review_get(
                github,
                &format!("pulls/{}", declared.run.pull_request),
                token,
            )
            .await?;
        let current_head = pull["head"]["sha"].as_str().ok_or("review_head_invalid")?;
        let base = pull["base"]["sha"].as_str().ok_or("review_base_invalid")?;
        if pull["base"]["repo"]["id"]
            .as_u64()
            .map(|id| id.to_string())
            .as_deref()
            != Some(&github.repository_id)
        {
            return Err("review_repository_mismatch");
        }
        let expected_run = assessment::RunIdentity {
            repository_id: github.repository_id.clone(),
            pull_request: declared.run.pull_request,
            head: declared.run.head.clone(),
            base: declared.run.base.clone(),
            profile_revision: revision.id,
            workflow_id: run["workflow_id"]
                .as_u64()
                .ok_or("review_workflow_invalid")?,
            run_id,
            attempt,
            workflow_sha,
            producer_digest: policy
                .producer_image
                .split_once('@')
                .ok_or("review_producer_invalid")?
                .1
                .into(),
        };
        self.verify_review_contract(github, token, current_head, &declared, (&contract, &lock))
            .await?;
        let (mut manifest, summary) = assessment::validate(
            &manifest_bytes,
            &artifacts,
            &Admission {
                run: &expected_run,
                runtime_digest: &artifact.resolved_digest,
                lock_fingerprint: &format!("sha256:{}", configuration.lock_fingerprint),
                contract_digest: &assessment::sha256(&contract),
                current_head,
                platform: "linux/amd64",
                ayni_version: &configuration.ayni_version,
            },
        )?;
        if run["conclusion"] == "cancelled" {
            manifest.outcome = assessment::Outcome::Cancelled;
        }
        if run["conclusion"] == "timed_out" {
            manifest.outcome = assessment::Outcome::InfrastructureFailed;
        }
        if manifest.run.head == current_head && manifest.run.base != base {
            manifest.outcome = assessment::Outcome::Stale;
        }
        let (change, assessment) = self.persist_review_import(
            repository,
            github,
            &pull,
            &(manifest_bytes, artifacts),
            &manifest,
            &summary,
        )?;
        self.reconcile_readiness(policy, github, token, &change, &assessment, &pull)
            .await
    }
}

pub(super) fn provider_review_state(pull: &serde_json::Value) -> v1::ChangeRequestState {
    if pull["merged_at"].as_str().is_some() {
        v1::ChangeRequestState::Merged
    } else if pull["state"] != "open" {
        v1::ChangeRequestState::Closed
    } else if pull["draft"] == true {
        v1::ChangeRequestState::Draft
    } else {
        v1::ChangeRequestState::Open
    }
}

pub(super) type EvidenceBundle = (Vec<u8>, BTreeMap<String, Vec<u8>>);

pub(super) fn unpack_assessment(archive: &[u8]) -> Result<EvidenceBundle, &'static str> {
    use std::io::{Cursor, Read};
    let mut zip =
        zip::ZipArchive::new(Cursor::new(archive)).map_err(|_| "review_archive_invalid")?;
    if zip.is_empty() || zip.len() > 3 {
        return Err("review_artifact_limit");
    }
    let mut files = BTreeMap::new();
    let mut total = 0_usize;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|_| "review_archive_invalid")?;
        let name = entry.name().to_owned();
        if !matches!(
            name.as_str(),
            "manifest.json" | "signals.json" | "runtime.json"
        ) || entry.is_dir()
            || entry.is_symlink()
            || entry.size() > assessment::MAX_BUNDLE_BYTES as u64
            || files.contains_key(&name)
        {
            return Err("review_artifact_invalid");
        }
        let mut bytes = Vec::new();
        entry
            .by_ref()
            .take(assessment::MAX_BUNDLE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "review_archive_invalid")?;
        total += bytes.len();
        if total > assessment::MAX_BUNDLE_BYTES {
            return Err("review_artifact_limit");
        }
        files.insert(name, bytes);
    }
    let manifest = files
        .remove("manifest.json")
        .ok_or("review_manifest_missing")?;
    Ok((manifest, files))
}

#[path = "readiness.rs"]
mod runtime_readiness;

#[cfg(test)]
mod review_archive_tests {
    use super::*;
    use std::io::{Cursor, Write};

    pub(super) fn archive(files: &[(&str, &[u8])]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in files {
            writer.start_file(*name, zip::write::SimpleFileOptions::default())?;
            writer.write_all(bytes)?;
        }
        Ok(writer.finish()?.into_inner())
    }

    #[test]
    pub(super) fn retains_exact_bytes_and_rejects_unexpected_archive_paths()
    -> Result<(), Box<dyn std::error::Error>> {
        let manifest = b"{\"version\":\"assessment-v1\"}\n";
        let (actual, files) = unpack_assessment(&archive(&[
            ("manifest.json", manifest),
            ("signals.json", b"original\n"),
        ])?)?;
        assert_eq!(actual, manifest);
        assert_eq!(files["signals.json"], b"original\n");
        for path in [
            "../manifest.json",
            "/manifest.json",
            "nested/manifest.json",
            "execution.json",
        ] {
            assert!(unpack_assessment(&archive(&[(path, manifest)])?).is_err());
        }
        assert!(unpack_assessment(&archive(&[("signals.json", b"missing manifest")])?).is_err());
        Ok(())
    }

    #[test]
    pub(super) fn enforces_total_extracted_size() -> Result<(), Box<dyn std::error::Error>> {
        let bytes = vec![b'x'; assessment::MAX_BUNDLE_BYTES];
        assert!(
            unpack_assessment(&archive(&[
                ("manifest.json", b"{}"),
                ("signals.json", &bytes)
            ])?)
            .is_err()
        );
        Ok(())
    }
}
