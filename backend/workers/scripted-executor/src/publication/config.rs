use super::Result;
use ayni_orchestrator_events::task_publication::PublicationCommand;
use std::{env, fs, path::PathBuf};

pub struct Configuration {
    pub authority: String,
    pub authority_token: String,
    pub github_token: String,
    pub workspace_id: String,
    pub repository: String,
    pub owner: String,
    pub name: String,
    pub pod_uid: String,
    pub api: String,
    pub remote: String,
    pub fixture: bool,
    pub scratch: PathBuf,
}
fn required(name: &str) -> Result<String> {
    env::var(name).map_err(|_| "publication configuration missing".into())
}
fn token(name: &str) -> Result<String> {
    let path = required(name)?;
    let value = fs::read_to_string(path).map_err(|_| "publication capability unavailable")?;
    let value = value.trim();
    if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err("publication capability invalid".into());
    }
    Ok(value.into())
}
fn http_url(value: &str, production: bool) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(value).map_err(|_| "publication endpoint invalid")?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
        || (production && url.scheme() != "https")
        || !matches!(url.scheme(), "http" | "https")
    {
        return Err("publication endpoint invalid".into());
    }
    Ok(url)
}
pub fn repository(value: &str) -> Result<(String, String)> {
    let canonical = ayni_orchestrator_events::task_publication::github_repository(value)
        .filter(|canonical| canonical == value)
        .ok_or("publication repository invalid")?;
    let suffix = canonical
        .strip_prefix("https://github.com/")
        .and_then(|v| v.strip_suffix(".git"))
        .ok_or("publication repository invalid")?;
    let (owner, name) = suffix
        .split_once('/')
        .ok_or("publication repository invalid")?;
    Ok((owner.into(), name.into()))
}
impl Configuration {
    pub fn load() -> Result<Self> {
        let repository = required("ORCHESTRATOR_PUBLICATION_REPOSITORY")?;
        let (owner, name) = self::repository(&repository)?;
        let workspace = required("ORCHESTRATOR_PUBLICATION_WORKSPACE_ID")?;
        uuid::Uuid::parse_str(&workspace).map_err(|_| "publication workspace invalid")?;
        let authority = required("ORCHESTRATOR_PUBLICATION_AUTHORITY_URL")?;
        http_url(&authority, false)?;
        let fixture = env::var("ORCHESTRATOR_PUBLICATION_FIXTURE_ENABLED").as_deref() == Ok("true");
        let (api, remote) = if fixture {
            let api = required("ORCHESTRATOR_PUBLICATION_FIXTURE_API_URL")?;
            http_url(&api, false)?;
            let remote = required("ORCHESTRATOR_PUBLICATION_FIXTURE_GIT_URL")?;
            let parsed =
                reqwest::Url::parse(&remote).map_err(|_| "fixture Git endpoint invalid")?;
            if parsed.scheme() != "git"
                || parsed.host_str().is_none()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.query().is_some()
                || parsed.fragment().is_some()
                || parsed.path() != format!("/{owner}/{name}.git")
            {
                return Err("fixture Git endpoint invalid".into());
            }
            (api, remote)
        } else {
            ("https://api.github.com".into(), repository.clone())
        };
        let pod_uid = required("ORCHESTRATOR_PUBLICATION_POD_UID")?;
        uuid::Uuid::parse_str(&pod_uid).map_err(|_| "publication Pod identity invalid")?;
        Ok(Self {
            authority,
            authority_token: token("ORCHESTRATOR_PUBLICATION_AUTHORITY_TOKEN_PATH")?,
            github_token: token("ORCHESTRATOR_PUBLICATION_GITHUB_TOKEN_PATH")?,
            workspace_id: workspace,
            repository,
            owner,
            name,
            pod_uid,
            api,
            remote,
            fixture,
            scratch: "/tmp/publications".into(),
        })
    }
    pub fn validate(&self, command: &PublicationCommand) -> Result {
        if command.repository != self.repository
            || command.workspace_id != self.workspace_id
            || command.patch.len() > 60 * 1024
            || command.title.len() > ayni_orchestrator_events::task_publication::TITLE_MAX_BYTES
            || command.body.len() > 24 * 1024
            || command.commit_message.len() > 8192
            || command.commit_timestamp <= 0
            || command.author_name != "Ayni Orchestrator"
            || command.author_email != "orchestrator@ayni.build"
            || command.branch != format!("task/{}", command.attempt_id)
            || !branch(&command.base_branch)
        {
            return Err("publication command invalid".into());
        }
        for id in [
            &command.publication_id,
            &command.attempt_id,
            &command.task_id,
            &command.workspace_id,
            &command.project_id,
            &command.conversation_id,
        ] {
            uuid::Uuid::parse_str(id).map_err(|_| "publication scope invalid")?;
        }
        if command.generation == 0 || !sha(&command.source_commit) {
            return Err("publication command identity invalid".into());
        }
        Ok(())
    }
}
pub fn sha(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn branch(value: &str) -> bool {
    ayni_orchestrator_events::task_publication::base_branch(&format!("refs/heads/{value}"))
        .as_deref()
        == Some(value)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repository_and_branch_reject_credentials_and_git_control_syntax() {
        assert!(repository("https://github.com/ayni-oss/orchestrator.git").is_ok());
        for value in [
            "https://token@github.com/a/b.git",
            "https://github.com/a/../b.git",
            "https://github.com/a/b.git?token=x",
            "http://github.com/a/b.git",
            "https://github.com/a/b/c.git",
        ] {
            assert!(repository(value).is_err());
        }
        for value in [
            "main.lock",
            "x/../main",
            "-main",
            "x@{1}",
            ".hidden",
            "main:other",
        ] {
            assert!(!branch(value));
        }
    }
}
