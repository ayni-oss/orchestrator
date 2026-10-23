//! Publication authority is separate from coding and managed validation.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationCommand {
    pub publication_id: String,
    pub attempt_id: String,
    pub task_id: String,
    pub workspace_id: String,
    pub project_id: String,
    pub conversation_id: String,
    pub generation: u64,
    pub repository: String,
    pub base_branch: String,
    pub branch: String,
    pub source_commit: String,
    pub source_pack_digest: String,
    pub lock_digest: String,
    pub patch: String,
    pub quality_digest: String,
    pub patch_digest: String,
    pub title: String,
    pub body: String,
    pub author_name: String,
    pub author_email: String,
    pub commit_message: String,
    pub commit_timestamp: i64,
    pub deadline_seconds: i64,
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationOutcome {
    pub state: String,
    pub safe_reason: String,
    pub commit_sha: String,
    pub pr_number: u64,
    pub pr_url: String,
    pub pr_state: String,
}
impl PublicationOutcome {
    #[must_use]
    pub fn validate(&self, command: &PublicationCommand, expected_commit: &str) -> bool {
        if !matches!(self.state.as_str(), "draft" | "blocked" | "uncertain")
            || self.safe_reason.len() > 128
            || !self
                .safe_reason
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c == b'_')
            || (!self.commit_sha.is_empty() && self.commit_sha != expected_commit)
        {
            return false;
        }
        if self.state == "draft" {
            let repository = command
                .repository
                .strip_suffix(".git")
                .unwrap_or(&command.repository);
            matches!(
                self.pr_state.as_str(),
                "draft" | "open" | "closed" | "merged"
            ) && self.safe_reason.is_empty()
                && oid(expected_commit)
                && self.commit_sha == expected_commit
                && self.pr_number > 0
                && self.pr_url == format!("{repository}/pull/{}", self.pr_number)
        } else {
            !self.safe_reason.is_empty()
                && self.pr_number == 0
                && self.pr_url.is_empty()
                && self.pr_state.is_empty()
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PublicationRun {
    pub command: PublicationCommand,
    pub state: String,
    pub generation: u64,
    pub pod_uid: String,
    pub lease_seconds: i64,
    pub commit_sha: String,
    pub tree_sha: String,
    pub outcome: Option<PublicationOutcome>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum PublicationSupervisorRequest {
    Poll {
        workspace_id: String,
        repository: String,
        pod_uid: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum PublicationWorkerRequest {
    Input,
    Prepared {
        commit_sha: String,
        tree_sha: String,
    },
    BeginCreate,
    Report {
        outcome: PublicationOutcome,
    },
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PublicationWorkerResponse {
    pub command: Option<PublicationCommand>,
    pub source_pack: Vec<u8>,
    pub run: Option<PublicationRun>,
    pub authorized: bool,
}
#[must_use]
pub fn oid(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
/// The initial publisher supports only the public GitHub HTTPS repository profile.
#[must_use]
pub fn github_repository(value: &str) -> Option<String> {
    let path = value.strip_prefix("https://github.com/")?;
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, name) = path.split_once('/')?;
    let segment = |s: &str| {
        !s.is_empty()
            && s != "."
            && s != ".."
            && s.bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
    };
    (owner.len() <= 39 && name.len() <= 100 && segment(owner) && segment(name))
        .then(|| format!("https://github.com/{owner}/{name}.git"))
}
#[must_use]
pub fn base_branch(value: &str) -> Option<String> {
    let branch = value.strip_prefix("refs/heads/")?;
    let valid = !branch.is_empty()
        && branch.len() <= 200
        && !branch.starts_with('-')
        && !branch.contains("..")
        && !branch.contains("//")
        && !branch.ends_with('.')
        && branch.split('/').all(|s| {
            !s.is_empty() && !s.starts_with('.') && !s.to_ascii_lowercase().ends_with(".lock")
        })
        && branch
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-/ .".contains(&c))
        && !branch.contains(' ');
    valid.then(|| branch.to_owned())
}

/// Maximum UTF-8 bytes accepted for a publication title by every component.
pub const TITLE_MAX_BYTES: usize = 256;

/// Publication copy is derived only from the task details the operator approved.
#[must_use]
pub fn title(objective: &str) -> String {
    let mut title = String::from("feat(tasks): ");
    for character in objective
        .lines()
        .next()
        .unwrap_or("Complete reviewed task")
        .chars()
        .filter(|c| !c.is_control())
        .take(145)
    {
        if title.len() + character.len_utf8() > TITLE_MAX_BYTES {
            break;
        }
        title.push(character);
    }
    title
}
#[must_use]
pub fn body(
    command: &crate::task_execution::TaskCommand,
    quality: &str,
    patch: &str,
    publication: &str,
) -> String {
    let objective: String = command.objective.clone();
    let scope: String = command.scope.clone();
    let criteria = command
        .acceptance_criteria
        .iter()
        .map(|s| format!("- {s}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "{objective}\n\nScope: {scope}\n\nAcceptance criteria:\n{criteria}\n\nTask: {}\nAttempt: {}\nSource: {}\nPatch: {patch}\nManaged quality: {quality}\n\n<!-- ayni-publication:{publication} -->",
        command.task_id, command.attempt_id, command.source_commit
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn publication_title_keeps_complete_utf8_characters_within_the_shared_byte_limit() {
        for character in ['a', 'é', '界', '🦀'] {
            let objective = character.to_string().repeat(400);
            let actual = title(&objective);
            assert!(actual.len() <= TITLE_MAX_BYTES);
            assert!(actual.starts_with("feat(tasks): "));
            assert!(objective.starts_with(actual.trim_start_matches("feat(tasks): ")));
            assert_eq!(title(&format!("{objective}\nignored")), actual);
        }
        assert_eq!(title("Reviewed task"), "feat(tasks): Reviewed task");
        assert_eq!(title("kept\ttext"), "feat(tasks): kepttext");
    }
    #[test]
    fn repository_identity_and_refs_are_not_reinterpreted() {
        assert_eq!(
            github_repository("https://github.com/example/bar.git.git"),
            Some("https://github.com/example/bar.git.git".into())
        );
        for value in [
            "https://github.com/a/b/c",
            "https://github.com/a/b?token=x",
            "https://github.com/a/b\\c",
            "https://other.invalid/a/b",
            "git@github.com:a/b.git",
        ] {
            assert!(github_repository(value).is_none());
        }
        for value in [
            "refs/tags/main",
            "refs/heads/a@{1}",
            "refs/heads/../main",
            "refs/heads/a.lock",
            "refs/heads/.hidden",
            "refs/heads/a//b",
        ] {
            assert!(base_branch(value).is_none());
        }
        assert_eq!(
            base_branch("refs/heads/release/alpha"),
            Some("release/alpha".into())
        );
    }
}
