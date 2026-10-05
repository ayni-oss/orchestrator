//! Repository-scoped GitHub publication and bounded candidate bundle transport.
use super::SourceClient;
use ayni_orchestrator_application::publication::delivery::{
    Delivery, Provider, Published, ReviewState,
};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, time::Duration};
use thiserror::Error;

#[derive(Debug, Error)]
#[error("publication adapter failed: {code}")]
pub struct PublicationFailure {
    pub code: &'static str,
}
impl From<&'static str> for PublicationFailure {
    fn from(code: &'static str) -> Self {
        Self { code }
    }
}

pub struct GithubPublisher<'a> {
    pub source: &'a SourceClient,
    pub repository: &'a ayni_orchestrator_api::v1::GithubRepository,
    pub token: &'a str,
    pub output: PathBuf,
}

impl Provider for GithubPublisher<'_> {
    type Error = PublicationFailure;

    async fn find(&self, delivery: &Delivery) -> Result<Option<Published>, Self::Error> {
        self.source
            .find_pull(
                &self.repository.owner,
                &self.repository.name,
                &delivery.branch,
                &delivery.base_branch,
                self.token,
            )
            .await?
            .as_ref()
            .map(published)
            .transpose()
    }

    async fn push(&self, delivery: &Delivery) -> Result<(), Self::Error> {
        push_candidate(
            &self.output,
            &format!(
                "https://github.com/{}/{}.git",
                self.repository.owner, self.repository.name
            ),
            delivery.base_revision.as_str(),
            delivery.head_revision.as_str(),
            &delivery.branch,
            self.token,
        )
        .await
        .map_err(Into::into)
    }

    async fn create(&self, delivery: &Delivery) -> Result<Published, Self::Error> {
        let pull = self.source.create_pull(&self.repository.owner, &self.repository.name, self.token,
            &serde_json::json!({"head":delivery.branch,"base":delivery.base_branch,"title":delivery.title,"draft":true,"body":delivery.body}))
            .await?;
        published(&pull)
    }
}

fn published(pull: &serde_json::Value) -> Result<Published, PublicationFailure> {
    let string = |value: &serde_json::Value| {
        value.as_str().map(str::to_owned).ok_or(PublicationFailure {
            code: "publication_response_invalid",
        })
    };
    Ok(Published {
        number: pull["number"].as_u64().ok_or(PublicationFailure {
            code: "publication_response_invalid",
        })?,
        url: pull["html_url"]
            .as_str()
            .filter(|url| url.starts_with("https://github.com/") && url.len() <= 2048)
            .ok_or(PublicationFailure {
                code: "publication_response_invalid",
            })?
            .into(),
        state: if pull["merged_at"].as_str().is_some() {
            ReviewState::Merged
        } else if pull["state"] == "closed" {
            ReviewState::Closed
        } else if pull["draft"] == true {
            ReviewState::Draft
        } else {
            ReviewState::Open
        },
        head_revision: ayni_orchestrator_application::identity::GitRevision::parse(&string(
            &pull["head"]["sha"],
        )?)
        .map_err(|_| PublicationFailure {
            code: "publication_response_invalid",
        })?,
        branch: string(&pull["head"]["ref"])?,
        base_branch: string(&pull["base"]["ref"])?,
    })
}

fn prepare_bundle(
    directory: &std::path::Path,
    head: &str,
) -> Result<tempfile::TempDir, &'static str> {
    use ayni_orchestrator_runtime::{coding::CodingReport, runtime_execution::read_bounded};
    if head.len() != 40 || !head.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("publication_head_invalid");
    }
    let final_report = read_bounded(&directory.join("report.json"), 65536)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<CodingReport>(&bytes).ok())
        .filter(|report| report.commit.as_deref() == Some(head));
    let (directory, report) = if let Some(report) = final_report {
        (directory.to_owned(), report)
    } else {
        // Recovery generations remain on the read-only output volume. Never
        // rewrite worker output or substitute a newer checkpoint for this head.
        let checkpoint = directory.join("checkpoints").join(head);
        let report = serde_json::from_slice::<CodingReport>(
            &read_bounded(&checkpoint.join("report.json"), 65536)
                .map_err(|_| "publication_report_unavailable")?,
        )
        .map_err(|_| "publication_report_invalid")?;
        (checkpoint, report)
    };
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
    Ok(temporary)
}

async fn push_candidate(
    directory: &std::path::Path,
    repository: &str,
    base: &str,
    head: &str,
    branch: &str,
    token: &str,
) -> Result<(), &'static str> {
    let output = directory.to_owned();
    let expected_head = head.to_owned();
    let temporary = tokio::task::spawn_blocking(move || prepare_bundle(&output, &expected_head))
        .await
        .map_err(|_| "publication_scratch_unavailable")??;
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
