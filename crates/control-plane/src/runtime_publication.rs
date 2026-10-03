impl RuntimeReconciler {
    async fn publish_candidates(&self) -> Result<(), &'static str> {
        use prost::Message;
        let actions = self
            .state
            .store
            .lock()
            .map_err(|_| "runtime_store_unavailable")?
            .pending_external_actions("PublishCandidate")
            .map_err(|_| "publication_state_invalid")?;
        for action in actions.into_iter().take(self.configuration.concurrency) {
            let request = v1::PublishCandidateRequest::decode(action.request_payload.as_slice())
                .map_err(|_| "publication_request_invalid")?;
            let (candidate, change, repository, attempt) = self.publication_inputs(
                &request.candidate_id,
                action
                    .target_id
                    .as_deref()
                    .ok_or("publication_identity_missing")?,
            )?;
            let profile = attempt
                .snapshot
                .as_ref()
                .and_then(|s| s.runtime.as_ref())
                .ok_or("coding_snapshot_missing")?;
            if !self
                .configuration
                .profiles
                .contains_key(&profile.runtime_profile_id)
            {
                continue;
            }
            if candidate.state != v1::CandidateState::Available as i32 {
                continue;
            }
            let Some(v1::repository_provider::Provider::Github(github)) =
                repository.provider.and_then(|p| p.provider)
            else {
                return Err("repository_provider_unsupported");
            };
            let branch = format!("agent/candidate-{:x}", Sha256::digest(&candidate.id));
            let token = self
                .source
                .publication_token(github.installation_id, &github.repository_id)
                .await?;
            // Read before writing, including after an uncertain POST or controller restart.
            let pull = if let Some(pull) = self
                .source
                .find_pull(
                    &github.owner,
                    &github.name,
                    &branch,
                    &request.base_branch,
                    &token,
                )
                .await?
            {
                pull
            } else {
                let directory =
                    std::path::Path::new("/runtime-outputs").join(operation_name(&attempt.id, 0));
                push_candidate(
                    &directory,
                    &format!("https://github.com/{}/{}.git", github.owner, github.name),
                    &candidate.base_revision,
                    &change.head_revision,
                    &branch,
                    &token,
                )
                .await?;
                self.source.create_pull(&github.owner, &github.name, &token, &serde_json::json!({"head":branch,"base":request.base_branch,"title":request.title,"draft":true,"body":format!("{}\n\n{}\n\n<!-- orchestrator-candidate:{} -->",request.body,candidate.coding_advisory.as_deref().unwrap_or("Coding-time checks are advisory; no assessment is available."),candidate.id)})).await?
            };
            let (provider, state) = publication_identity(
                &pull,
                github.repository_id,
                &change.head_revision,
                &branch,
                &request.base_branch,
            )?;
            self.state
                .store
                .lock()
                .map_err(|_| "runtime_store_unavailable")?
                .reconcile_change_request(
                    &action.idempotency_key,
                    change.resource_version,
                    provider,
                    state,
                    &change.head_revision,
                    None,
                )
                .map_err(|_| "publication_state_changed")?;
        }
        Ok(())
    }
    fn publication_inputs(
        &self,
        candidate_id: &str,
        change_id: &str,
    ) -> Result<
        (
            v1::Candidate,
            v1::ChangeRequest,
            v1::RepositoryConnection,
            v1::DevelopmentAttempt,
        ),
        &'static str,
    > {
        let store = self
            .state
            .store
            .lock()
            .map_err(|_| "runtime_store_unavailable")?;
        let candidate = store
            .candidate(candidate_id)
            .map_err(|_| "publication_candidate_missing")?;
        let inputs = (
            candidate.clone(),
            store
                .change_request(change_id)
                .map_err(|_| "publication_identity_missing")?,
            store
                .repository_connection(&candidate.repository_connection_id)
                .map_err(|_| "repository_missing")?,
            store
                .development_attempt(&candidate.development_attempt_id)
                .map_err(|_| "coding_snapshot_missing")?,
        );
        drop(store);
        Ok(inputs)
    }
}

async fn push_candidate(
    directory: &std::path::Path,
    repository: &str,
    base: &str,
    head: &str,
    branch: &str,
    token: &str,
) -> Result<(), &'static str> {
    use ayni_orchestrator_runtime::{coding::CodingReport, runtime_execution::read_bounded};
    let report: CodingReport = serde_json::from_slice(
        &read_bounded(&directory.join("report.json"), 65536)
            .map_err(|_| "publication_report_unavailable")?,
    )
    .map_err(|_| "publication_report_invalid")?;
    let bundle = read_bounded(&directory.join("candidate.bundle"), 64 * 1024 * 1024)
        .map_err(|_| "publication_bundle_unavailable")?;
    if report.commit.as_deref() != Some(head)
        || report.bundle_digest.as_deref()
            != Some(format!("sha256:{:x}", Sha256::digest(&bundle)).as_str())
    {
        return Err("publication_bundle_mismatch");
    }
    // Only a freshly created bare object database is used. Never execute hooks,
    // check out candidate files, or consume its Git configuration in the controller.
    let temporary = tempfile::tempdir().map_err(|_| "publication_scratch_unavailable")?;
    std::fs::write(temporary.path().join("candidate.bundle"), bundle)
        .map_err(|_| "publication_scratch_unavailable")?;
    publication_git(temporary.path(), &["init", "--bare", "objects.git"], None).await?;
    let objects = temporary.path().join("objects.git");
    publication_git(
        &objects,
        &[
            "fetch",
            "--quiet",
            "--depth=1",
            "--no-tags",
            "--",
            repository,
            base,
        ],
        Some(token),
    )
    .await?;
    publication_git(
        &objects,
        &["fetch", "--quiet", "--", "../candidate.bundle", "HEAD"],
        None,
    )
    .await?;
    if publication_git(&objects, &["rev-parse", "FETCH_HEAD"], None)
        .await?
        .trim()
        != head
    {
        return Err("publication_commit_mismatch");
    }
    publication_git(&objects, &["merge-base", "--is-ancestor", base, head], None).await?;
    let reference = format!("refs/heads/{branch}");
    let existing = publication_git(
        &objects,
        &["ls-remote", "--heads", "--", repository, &reference],
        Some(token),
    )
    .await?;
    if !existing.is_empty() {
        if existing.split_whitespace().next() != Some(head) {
            return Err("publication_branch_conflict");
        }
        return Ok(());
    }
    publication_git(
        &objects,
        &[
            "push",
            "--quiet",
            &format!("--force-with-lease={reference}:"),
            "--",
            repository,
            &format!("{head}:{reference}"),
        ],
        Some(token),
    )
    .await?;
    Ok(())
}

async fn publication_git(
    directory: &std::path::Path,
    args: &[&str],
    token: Option<&str>,
) -> Result<String, &'static str> {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use tokio::io::AsyncReadExt;
    let mut command = tokio::process::Command::new("git");
    command
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "credential.helper=",
            "-c",
            "gc.auto=0",
        ])
        .args(args)
        .current_dir(directory)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", directory)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true);
    if let Some(token) = token {
        command
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "http.https://github.com/.extraheader")
            .env(
                "GIT_CONFIG_VALUE_0",
                format!(
                    "AUTHORIZATION: basic {}",
                    STANDARD.encode(format!("x-access-token:{token}"))
                ),
            );
    }
    let mut child = command.spawn().map_err(|_| "publication_git_unavailable")?;
    let mut output = child
        .stdout
        .take()
        .ok_or("publication_git_unavailable")?
        .take(4097);
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(120), async {
        output
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| "publication_git_failed")?;
        if bytes.len() > 4096 {
            return Err("publication_git_output_oversized");
        }
        if !child
            .wait()
            .await
            .map_err(|_| "publication_git_failed")?
            .success()
        {
            return Err("publication_git_failed");
        }
        String::from_utf8(bytes).map_err(|_| "publication_git_failed")
    })
    .await
    .map_err(|_| "publication_git_timeout")?
}

fn publication_identity(
    pull: &serde_json::Value,
    repository_id: String,
    head: &str,
    branch: &str,
    base: &str,
) -> Result<(v1::ChangeRequestProvider, v1::ChangeRequestState), &'static str> {
    if pull["head"]["sha"].as_str() != Some(head)
        || pull["head"]["ref"].as_str() != Some(branch)
        || pull["base"]["ref"].as_str() != Some(base)
    {
        return Err("publication_identity_conflict");
    }
    let provider = v1::ChangeRequestProvider {
        provider: Some(v1::change_request_provider::Provider::Github(
            v1::GithubChangeRequest {
                repository_id,
                pull_request_number: pull["number"]
                    .as_u64()
                    .ok_or("publication_response_invalid")?,
                html_url: pull["html_url"]
                    .as_str()
                    .filter(|url| url.starts_with("https://github.com/") && url.len() <= 2048)
                    .ok_or("publication_response_invalid")?
                    .into(),
            },
        )),
    };
    let state = if pull["merged_at"].as_str().is_some() {
        v1::ChangeRequestState::Merged
    } else if pull["state"] == "closed" {
        v1::ChangeRequestState::Closed
    } else if pull["draft"] == true {
        v1::ChangeRequestState::Draft
    } else {
        v1::ChangeRequestState::Open
    };
    Ok((provider, state))
}
