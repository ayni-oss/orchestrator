//! GitHub App operations for the one private sandbox repository.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    fs, io,
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::development::{DevelopmentTask, RepositoryBinding};

#[derive(Clone)]
pub struct GitHubApp {
    app_id: u64,
    private_key: Vec<u8>,
    client: Client,
}

#[derive(Serialize)]
struct Claims {
    iat: u64,
    exp: u64,
    iss: String,
}

#[derive(Deserialize)]
struct InstallationToken {
    token: String,
}

#[derive(Deserialize)]
struct PullRequest {
    number: u64,
    html_url: String,
    draft: bool,
    state: String,
    head: PullHead,
}

impl PullRequest {
    fn verified(self, candidate: &str) -> Result<(u64, String), String> {
        if self.head.sha != candidate || !self.draft || self.state != "open" {
            return Err("task PR is not an open draft for the frozen candidate".into());
        }
        Ok((self.number, self.html_url))
    }
}

#[derive(Deserialize)]
struct PullHead {
    sha: String,
}

impl GitHubApp {
    pub fn new(app_id: u64, private_key_path: &Path) -> Result<Self, String> {
        if app_id == 0 {
            return Err("GitHub App ID must be positive".into());
        }
        let private_key = fs::read(private_key_path)
            .map_err(|error| format!("could not read GitHub App private key: {error}"))?;
        EncodingKey::from_rsa_pem(&private_key)
            .map_err(|_| "GitHub App private key is not valid RSA PEM".to_owned())?;
        Ok(Self {
            app_id,
            private_key,
            client: Client::new(),
        })
    }

    fn app_jwt(&self) -> Result<String, String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "clock precedes Unix epoch".to_owned())?
            .as_secs();
        let claims = Claims {
            iat: now.saturating_sub(60),
            exp: now.saturating_add(540),
            iss: self.app_id.to_string(),
        };
        encode(
            &Header::new(Algorithm::RS256),
            &claims,
            &EncodingKey::from_rsa_pem(&self.private_key).map_err(|_| "invalid app key")?,
        )
        .map_err(|_| "could not sign GitHub App JWT".into())
    }

    pub async fn installation_token(
        &self,
        repository: &RepositoryBinding,
    ) -> Result<String, String> {
        self.token_with_permissions(repository, true).await
    }

    pub async fn clone_token(&self, repository: &RepositoryBinding) -> Result<String, String> {
        self.token_with_permissions(repository, false).await
    }

    async fn token_with_permissions(
        &self,
        repository: &RepositoryBinding,
        publish: bool,
    ) -> Result<String, String> {
        if repository.installation_id <= 0 {
            return Err("invalid installation ID".into());
        }
        let permissions = if publish {
            json!({"contents": "write", "pull_requests": "write"})
        } else {
            json!({"contents": "read"})
        };
        let response = self
            .client
            .post(format!(
                "https://api.github.com/app/installations/{}/access_tokens",
                repository.installation_id
            ))
            .header("User-Agent", "ayni-orchestrator")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .bearer_auth(self.app_jwt()?)
            .json(&json!({"repositories": [repository.repository], "permissions": permissions}))
            .send()
            .await
            .map_err(|error| format!("GitHub token request failed: {error}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "GitHub token request returned {}",
                response.status()
            ));
        }
        let token: InstallationToken = response
            .json()
            .await
            .map_err(|_| "GitHub token response was invalid".to_owned())?;
        Ok(token.token)
    }

    /// Find the deterministic PR first; push the frozen commit and create it
    /// only when no matching PR exists. Retrying an uncertain response starts
    /// with the same lookup.
    pub async fn publish(
        &self,
        task: &DevelopmentTask,
        candidate_bundle: &Path,
        branch: &str,
        title: &str,
        body: &str,
    ) -> Result<(u64, String), String> {
        let candidate = task
            .candidate_commit
            .as_deref()
            .ok_or("task has no frozen candidate")?;
        let token = self.installation_token(&task.repository).await?;
        if let Some(pr) = self.find_pr(&token, &task.repository, branch).await? {
            return pr.verified(candidate);
        }
        push_bundle(
            candidate_bundle,
            candidate,
            &task.input.start_revision,
            branch,
            &token,
            &task.repository,
        )?;
        if let Some(pr) = self.find_pr(&token, &task.repository, branch).await? {
            return pr.verified(candidate);
        }
        self.create_pr(&token, &task.repository, branch, candidate, title, body)
            .await
    }

    async fn create_pr(
        &self,
        token: &str,
        repository: &RepositoryBinding,
        branch: &str,
        candidate: &str,
        title: &str,
        body: &str,
    ) -> Result<(u64, String), String> {
        let response = self
            .client
            .post(pulls_url(repository))
            .header("User-Agent", "ayni-orchestrator")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .bearer_auth(token)
            .json(&json!({"title": title, "body": body, "head": branch,
                "base": repository.base_branch, "draft": true}))
            .send()
            .await;
        match response {
            Ok(response) if response.status() == StatusCode::CREATED => {
                let pr: PullRequest = response
                    .json()
                    .await
                    .map_err(|_| "GitHub PR response was invalid".to_owned())?;
                pr.verified(candidate)
            }
            _ => {
                // A timeout or error may follow a successful GitHub create.
                let pr = self
                    .find_pr(token, repository, branch)
                    .await?
                    .ok_or("PR creation outcome is uncertain; retry will reconcile")?;
                pr.verified(candidate)
            }
        }
    }

    async fn find_pr(
        &self,
        token: &str,
        repository: &RepositoryBinding,
        branch: &str,
    ) -> Result<Option<PullRequest>, String> {
        let response = self
            .client
            .get(pulls_url(repository))
            .header("User-Agent", "ayni-orchestrator")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .bearer_auth(token)
            .query(&[
                ("head", format!("{}:{branch}", repository.owner)),
                ("base", repository.base_branch.clone()),
                ("state", "all".to_owned()),
            ])
            .send()
            .await
            .map_err(|error| format!("GitHub PR lookup failed: {error}"))?;
        if !response.status().is_success() {
            return Err(format!("GitHub PR lookup returned {}", response.status()));
        }
        let prs: Vec<PullRequest> = response
            .json()
            .await
            .map_err(|_| "GitHub PR lookup response was invalid".to_owned())?;
        if prs.len() > 1 {
            return Err("more than one PR exists for task branch".into());
        }
        Ok(prs.into_iter().next())
    }
}

fn push_bundle(
    bundle: &Path,
    candidate: &str,
    start: &str,
    branch: &str,
    token: &str,
    repository: &RepositoryBinding,
) -> Result<(), String> {
    let directory =
        tempfile::tempdir().map_err(|error| format!("scratch creation failed: {error}"))?;
    git(directory.path(), &["init", "--quiet"], None)?;
    git(
        directory.path(),
        &[
            "fetch",
            bundle.to_str().ok_or("invalid bundle path")?,
            "HEAD",
        ],
        None,
    )?;
    git(
        directory.path(),
        &["merge-base", "--is-ancestor", start, candidate],
        None,
    )?;
    let refspec = format!("{candidate}:refs/heads/{branch}");
    git(
        directory.path(),
        &[
            "push",
            &format!(
                "https://github.com/{}/{}.git",
                repository.owner, repository.repository
            ),
            &refspec,
        ],
        Some(token),
    )?;
    Ok(())
}

fn pulls_url(repository: &RepositoryBinding) -> String {
    format!(
        "https://api.github.com/repos/{}/{}/pulls",
        repository.owner, repository.repository
    )
}

fn git(directory: &Path, args: &[&str], token: Option<&str>) -> Result<(), String> {
    let mut command = Command::new("git");
    command
        .current_dir(directory)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0");
    if let Some(token) = token {
        let basic = STANDARD.encode(format!("x-access-token:{token}"));
        command
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "http.https://github.com/.extraheader")
            .env(
                "GIT_CONFIG_VALUE_0",
                format!("Authorization: Basic {basic}"),
            );
    }
    let output = command
        .output()
        .map_err(|error: io::Error| format!("git execution failed: {error}"))?;
    if !output.status.success() {
        // Git stderr may contain a remote URL or sensitive server text.
        return Err(format!("git operation failed with {}", output.status));
    }
    Ok(())
}
