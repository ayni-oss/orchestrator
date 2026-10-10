//! Worker-owned GitHub draft delivery with durable intent and uncertain-response recovery.
use super::{recovery, sessions::Context};
use crate::operations::runtime::command;
use ayni_orchestrator_runtime::{
    coding::{PublicationOutcome, PullRequestReceipt, WorkerResult},
    runtime_execution::read_bounded,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{io, path::Path, time::Duration};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    task_id: String,
    attempt_id: String,
    repository: String,
    branch: String,
    base_branch: String,
    head_commit: String,
    receipt: Option<PullRequestReceipt>,
}
fn intent(context: &Context, result: &WorkerResult) -> io::Result<Journal> {
    Ok(Journal {
        task_id: context.spec.task_id.clone(),
        attempt_id: context.spec.attempt_id.clone(),
        repository: context.spec.publication.repository.clone(),
        branch: context.spec.publication.branch.clone(),
        base_branch: context.spec.publication.base_branch.clone(),
        head_commit: result
            .commit
            .clone()
            .ok_or_else(|| io::Error::other("publication has no final commit"))?,
        receipt: None,
    })
}
fn marker(journal: &Journal) -> String {
    format!(
        "<!-- ayni-task:{} attempt:{} -->",
        journal.task_id, journal.attempt_id
    )
}
pub(super) fn token() -> io::Result<String> {
    let bytes = projected_credential(Path::new("/source"), "token")?;
    let value = std::str::from_utf8(&bytes)
        .map_err(io::Error::other)?
        .trim();
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(io::Error::other("attempt provider credential unavailable"));
    }
    Ok(value.into())
}
pub(super) fn projected_credential(directory: &Path, name: &str) -> io::Result<Vec<u8>> {
    let root = directory.canonicalize()?;
    let path = directory.join(name).canonicalize()?;
    if !path.starts_with(root) {
        return Err(io::Error::other(
            "credential projection leaves its trusted mount",
        ));
    }
    read_bounded(&path, 16384)
}
async fn api(
    context: &Context,
    method: reqwest::Method,
    endpoint: &str,
    query: &[(&str, String)],
    body: Option<Value>,
) -> io::Result<Option<Value>> {
    let client = reqwest::Client::builder()
        .timeout(context.remaining()?.min(Duration::from_secs(30)))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| io::Error::other("provider client unavailable"))?;
    let mut request = client
        .request(
            method,
            format!(
                "https://api.github.com/repos/{}/{endpoint}",
                context.spec.publication.repository
            ),
        )
        .header("User-Agent", "ayni-runtime-worker")
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .bearer_auth(token()?)
        .query(query);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let mut response = request
        .send()
        .await
        .map_err(|_| io::Error::other("provider response uncertain"))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(io::Error::other("provider operation unavailable"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| io::Error::other("provider response incomplete"))?
    {
        if bytes.len() + chunk.len() > 1_048_576 {
            return Err(io::Error::other("provider response exceeds bound"));
        }
        bytes.extend(chunk);
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| io::Error::other("provider response invalid"))
}
fn receipt(journal: &Journal, pull: &Value) -> io::Result<PullRequestReceipt> {
    if pull["head"]["repo"]["full_name"] != journal.repository
        || pull["base"]["repo"]["full_name"] != journal.repository
        || pull["head"]["ref"] != journal.branch
        || pull["base"]["ref"] != journal.base_branch
        || pull["head"]["sha"] != journal.head_commit
        || !pull["body"]
            .as_str()
            .is_some_and(|v| v.contains(&marker(journal)))
        || pull["draft"] != true
        || pull["state"] != "open"
    {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "pull request identity or head conflicts with frozen delivery",
        ));
    }
    let number = pull["number"]
        .as_u64()
        .filter(|v| *v > 0)
        .ok_or_else(|| io::Error::other("pull request number missing"))?;
    let url = format!("https://github.com/{}/pull/{number}", journal.repository);
    if pull["html_url"] != url {
        return Err(io::Error::other("pull request URL identity mismatch"));
    }
    Ok(PullRequestReceipt {
        repository: journal.repository.clone(),
        number,
        url,
        branch: journal.branch.clone(),
        base_branch: journal.base_branch.clone(),
        head_commit: journal.head_commit.clone(),
        draft: true,
    })
}
async fn find(context: &Context, journal: &Journal) -> io::Result<Option<PullRequestReceipt>> {
    let owner = journal
        .repository
        .split('/')
        .next()
        .ok_or_else(|| io::Error::other("repository owner missing"))?;
    let response = api(
        context,
        reqwest::Method::GET,
        "pulls",
        &[
            ("state", "all".into()),
            ("head", format!("{owner}:{}", journal.branch)),
            ("per_page", "100".into()),
        ],
        None,
    )
    .await?
    .ok_or_else(|| io::Error::other("repository pull requests unavailable"))?;
    let pulls = response
        .as_array()
        .ok_or_else(|| io::Error::other("pull request list invalid"))?;
    if pulls.len() > 1 {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "multiple pull requests conflict with delivery",
        ));
    }
    pulls.first().map(|pull| receipt(journal, pull)).transpose()
}
async fn push(context: &Context, journal: &Journal) -> io::Result<()> {
    let existing = api(
        context,
        reqwest::Method::GET,
        &format!("git/ref/heads/{}", journal.branch),
        &[],
        None,
    )
    .await?;
    if let Some(reference) = existing {
        if reference["object"]["sha"] != journal.head_commit {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "remote branch head conflicts with retained final work",
            ));
        }
        return Ok(());
    }
    reset_git_configuration(&context.workspace)?;
    let source_token = token()?;
    let deadline = context.remaining()?.min(Duration::from_secs(180));
    let mut process = tokio::process::Command::from(command("git", &context.workspace));
    // The empty lease permits creation only. A racing human head is rejected.
    process
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "push",
            "--quiet",
            &format!("--force-with-lease=refs/heads/{}:", journal.branch),
            "--",
            &context.spec.execution.repository_url,
            &format!("{}:refs/heads/{}", journal.head_commit, journal.branch),
        ])
        .env("GIT_CONFIG_COUNT", "2")
        .env("GIT_CONFIG_KEY_1", "http.https://github.com/.extraheader")
        .env(
            "GIT_CONFIG_VALUE_1",
            format!(
                "AUTHORIZATION: basic {}",
                STANDARD.encode(format!("x-access-token:{source_token}"))
            ),
        );
    if crate::operations::process::status(&mut process, deadline)
        .await?
        .success()
    {
        Ok(())
    } else {
        Err(io::Error::other("publication push outcome uncertain"))
    }
}

fn reset_git_configuration(workspace: &Path) -> io::Result<()> {
    let directory = workspace.join(".git");
    if !std::fs::symlink_metadata(&directory)?.is_dir() {
        return Err(io::Error::other("publication Git directory is unsafe"));
    }
    // Source commands may have changed local Git configuration. Credential-bearing
    // push must not execute their helpers, URL rewrites or included configuration.
    recovery::atomic_write(
        &directory.join("config"),
        b"[core]\nrepositoryformatversion = 0\nbare = false\nfilemode = true\nlogallrefupdates = true\nhooksPath = /dev/null\n",
    )
}

pub async fn deliver(context: &Context, result: &mut WorkerResult) -> io::Result<()> {
    let path = context.output.join("publication.json");
    let expected = intent(context, result)?;
    let mut journal = if path.exists() {
        recovery::read_json::<Journal>(&path)?
    } else {
        expected.clone()
    };
    if journal.task_id != expected.task_id
        || journal.attempt_id != expected.attempt_id
        || journal.repository != expected.repository
        || journal.branch != expected.branch
        || journal.base_branch != expected.base_branch
        || journal.head_commit != expected.head_commit
    {
        return Err(io::Error::other(
            "publication intent conflicts with frozen final work",
        ));
    }
    if let Some(receipt) = journal.receipt {
        if receipt.repository != journal.repository
            || receipt.branch != journal.branch
            || receipt.base_branch != journal.base_branch
            || receipt.head_commit != journal.head_commit
            || !receipt.draft
            || receipt.number == 0
            || receipt.url
                != format!(
                    "https://github.com/{}/pull/{}",
                    journal.repository, receipt.number
                )
        {
            return Err(io::Error::other("persisted receipt identity is invalid"));
        }
        result.pull_request = Some(receipt);
        result.publication = PublicationOutcome::Published;
        return Ok(());
    }
    recovery::write_json(&path, &journal)?;
    result.publication = PublicationOutcome::Publishing;
    context.state.lock().await.progress.publication = PublicationOutcome::Publishing;
    context.persist().await?;
    let mut last_error = None;
    for _ in 0..3 {
        context.remaining()?;
        match publish_once(context, &journal, result).await {
            Ok(pull) => {
                journal.receipt = Some(pull.clone());
                recovery::write_json(&path, &journal)?;
                result.pull_request = Some(pull);
                result.publication = PublicationOutcome::Published;
                return Ok(());
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                result.publication = PublicationOutcome::Conflict;
                return Err(error);
            }
            Err(error) => {
                last_error = Some(error);
            }
        }
    }
    result.publication = PublicationOutcome::Failed;
    Err(last_error.unwrap_or_else(|| io::Error::other("publication unavailable")))
}
async fn publish_once(
    context: &Context,
    journal: &Journal,
    result: &WorkerResult,
) -> io::Result<PullRequestReceipt> {
    if let Some(pull) = find(context, journal).await? {
        return Ok(pull);
    }
    push(context, journal).await?;
    let title = format!(
        "feat(task): {}",
        context
            .spec
            .request
            .lines()
            .next()
            .unwrap_or("implement requested change")
            .chars()
            .filter(|c| !c.is_control())
            .take(72)
            .collect::<String>()
    );
    let response = api(context, reqwest::Method::POST, "pulls", &[], Some(json!({"head": journal.branch, "base": journal.base_branch, "title": title, "body": description(context, journal, result), "draft":true}))).await;
    match response {
        Ok(Some(pull)) => receipt(journal, &pull),
        _ => find(context, journal)
            .await?
            .ok_or_else(|| io::Error::other("pull request creation outcome uncertain")),
    }
}
fn escape(value: &str) -> String {
    let mut result = String::new();
    for c in value.chars() {
        match c {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '\\' | '`' | '*' | '_' | '[' | ']' | '#' | '|' | '!' => {
                result.push('\\');
                result.push(c);
            }
            c if c.is_control() && c != '\n' => {}
            c => result.push(c),
        }
    }
    result
}
fn description(context: &Context, journal: &Journal, result: &WorkerResult) -> String {
    let known = |value: Option<u32>| value.map_or_else(|| "unavailable".into(), |v| v.to_string());
    let findings = result
        .quality
        .findings
        .iter()
        .take(32)
        .map(|v| format!("- {}", escape(v)))
        .collect::<Vec<_>>()
        .join("\n");
    let evidence = result
        .quality
        .evidence
        .as_ref()
        .map_or("unavailable", |v| v.signals_digest.as_str());
    format!(
        "{}\n\n## Request\n\n{}\n\n## Implementation\n\n{}\n\nFinal commit: `{}`\n\n## Full quality check\n\nThis worker-run final check is independent of agent tool calls described above. Its result is the authoritative quality status for this draft.\n\nOutcome: {:?}\n\nCompleted targets: {} · skipped: {} · warnings: {} · failures: {}\n\nCoding turns: {} / {} · repair sessions: {} / {}\n\nEvidence digest: `{}`\n\n{}",
        marker(journal),
        escape(&context.spec.request),
        escape(&context.state.try_lock().map_or_else(
            |_| "Requested changes retained in the final commit.".into(),
            |v| v.last_response.clone()
        )),
        journal.head_commit,
        result.quality.outcome,
        known(result.quality.completed_targets),
        known(result.quality.skipped_targets),
        known(result.quality.warning_offenders),
        known(result.quality.failing_offenders),
        result.coding_turns_consumed,
        context.spec.limits.coding_turns,
        result.quality_round,
        context.spec.limits.quality_repair_limit,
        evidence,
        findings
    )
}
#[cfg(test)]
mod tests;
