//! Bounded GitHub protocol. A create call is never retried by this adapter.
use super::{Result, config::Configuration};
use ayni_orchestrator_events::task_publication::{PublicationCommand, PublicationOutcome};
use reqwest::{Client, Response};
use serde::Deserialize;
use serde_json::json;
use std::time::Duration;

pub fn client() -> Result<Client> {
    Ok(Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "publication HTTP unavailable")?)
}
pub async fn bytes(mut response: Response, maximum: usize) -> Result<Vec<u8>> {
    if !response.status().is_success() {
        return Err("publication endpoint rejected operation".into());
    }
    let mut result = Vec::new();
    while let Some(part) = response
        .chunk()
        .await
        .map_err(|_| "publication response interrupted")?
    {
        if result.len().saturating_add(part.len()) > maximum {
            return Err("publication response limit".into());
        }
        result.extend_from_slice(&part);
    }
    Ok(result)
}
#[derive(Deserialize)]
struct Repository {
    full_name: String,
}
#[derive(Deserialize)]
struct Reference {
    #[serde(rename = "ref")]
    name: String,
    sha: String,
    repo: Option<Repository>,
}
#[derive(Deserialize)]
struct Pull {
    number: u64,
    html_url: String,
    state: String,
    draft: bool,
    body: Option<String>,
    head: Reference,
    base: Reference,
    #[serde(default)]
    merged: bool,
    merged_at: Option<String>,
}
fn endpoint(c: &Configuration) -> String {
    format!(
        "{}/repos/{}/{}/pulls",
        c.api.trim_end_matches('/'),
        c.owner,
        c.name
    )
}
fn request(
    http: &Client,
    c: &Configuration,
    method: reqwest::Method,
    url: &str,
) -> reqwest::RequestBuilder {
    http.request(method, url)
        .bearer_auth(&c.github_token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "ayni-orchestrator-publication")
}
fn accepted(pull: &Pull, c: &PublicationCommand, commit: &str) -> Result<PublicationOutcome> {
    let name = c
        .repository
        .strip_prefix("https://github.com/")
        .and_then(|v| v.strip_suffix(".git"))
        .ok_or("publication repository invalid")?;
    let marker = format!("<!-- ayni-publication:{} -->", c.publication_id);
    if pull.number == 0
        || pull.head.name != c.branch
        || pull.base.name != c.base_branch
        || pull.head.sha != commit
        || pull.head.repo.as_ref().is_none_or(|r| r.full_name != name)
        || pull.base.repo.as_ref().is_none_or(|r| r.full_name != name)
        || pull.body.as_ref().is_none_or(|b| !b.contains(&marker))
        || pull.html_url != format!("https://github.com/{name}/pull/{}", pull.number)
        || !matches!(pull.state.as_str(), "open" | "closed")
    {
        return Err("publication pull identity conflict".into());
    }
    let pr_state = if pull.merged || pull.merged_at.is_some() {
        "merged"
    } else if pull.state == "closed" {
        "closed"
    } else if pull.draft {
        "draft"
    } else {
        "open"
    };
    Ok(PublicationOutcome {
        state: "draft".into(),
        safe_reason: String::new(),
        commit_sha: commit.into(),
        pr_number: pull.number,
        pr_url: pull.html_url.clone(),
        pr_state: pr_state.into(),
    })
}
pub async fn reconcile(
    http: &Client,
    configuration: &Configuration,
    command: &PublicationCommand,
    commit: &str,
) -> Result<Option<PublicationOutcome>> {
    let response = request(
        http,
        configuration,
        reqwest::Method::GET,
        &endpoint(configuration),
    )
    .query(&[
        ("state", "all".to_owned()),
        (
            "head",
            format!("{}:{}", configuration.owner, command.branch),
        ),
        ("base", command.base_branch.clone()),
        ("per_page", "100".into()),
    ])
    .send()
    .await
    .map_err(|_| "publication reconciliation unavailable")?;
    let candidates: Vec<Pull> = serde_json::from_slice(&bytes(response, 256 * 1024).await?)
        .map_err(|_| "publication response invalid")?;
    if candidates.len() >= 100 {
        return Err("publication history limit".into());
    }
    let mut found = None;
    for candidate in candidates {
        let result = accepted(&candidate, command, commit)?;
        if found.is_some() {
            return Err("publication multiple matching pulls".into());
        }
        found = Some(result);
    }
    Ok(found)
}
pub async fn create(
    http: &Client,
    configuration: &Configuration,
    command: &PublicationCommand,
    commit: &str,
) -> Result<PublicationOutcome> {
    let response = request(http, configuration, reqwest::Method::POST, &endpoint(configuration))
        .json(&json!({"title":command.title,"body":command.body,"head":command.branch,"base":command.base_branch,"draft":true,"maintainer_can_modify":false}))
        .send().await.map_err(|_| "publication create response uncertain")?;
    let pull: Pull = serde_json::from_slice(&bytes(response, 256 * 1024).await?)
        .map_err(|_| "publication create response invalid")?;
    if !pull.draft || pull.state != "open" {
        return Err("publication create response uncertain".into());
    }
    accepted(&pull, command, commit)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn command() -> PublicationCommand {
        PublicationCommand {
            repository: "https://github.com/fixture/repo.git".into(),
            publication_id: "publication".into(),
            branch: "task/attempt".into(),
            base_branch: "main".into(),
            ..Default::default()
        }
    }
    fn pull() -> Pull {
        Pull {
            number: 1,
            html_url: "https://github.com/fixture/repo/pull/1".into(),
            state: "open".into(),
            draft: true,
            body: Some("Approved work\n<!-- ayni-publication:publication -->".into()),
            head: Reference {
                name: "task/attempt".into(),
                sha: "a".repeat(40),
                repo: Some(Repository {
                    full_name: "fixture/repo".into(),
                }),
            },
            base: Reference {
                name: "main".into(),
                sha: "b".repeat(40),
                repo: Some(Repository {
                    full_name: "fixture/repo".into(),
                }),
            },
            merged: false,
            merged_at: None,
        }
    }
    #[test]
    fn reconciliation_preserves_closed_or_merged_pull_identity_without_a_remote_branch() -> Result {
        let command = command();
        let mut pull = pull();
        pull.state = "closed".into();
        assert_eq!(
            accepted(&pull, &command, &"a".repeat(40))?.pr_state,
            "closed"
        );
        pull.merged = true;
        assert_eq!(
            accepted(&pull, &command, &"a".repeat(40))?.pr_state,
            "merged"
        );
        Ok(())
    }
    #[test]
    fn foreign_marker_repository_or_commit_cannot_be_adopted() {
        let command = command();
        let mut pull = pull();
        pull.body = Some("foreign".into());
        assert!(accepted(&pull, &command, &"a".repeat(40)).is_err());
        pull.body = Some("<!-- ayni-publication:publication -->".into());
        pull.head.sha = "c".repeat(40);
        assert!(accepted(&pull, &command, &"a".repeat(40)).is_err());
        pull.head.sha = "a".repeat(40);
        pull.head.repo = Some(Repository {
            full_name: "another/repo".into(),
        });
        assert!(accepted(&pull, &command, &"a".repeat(40)).is_err());
    }
}
