//! One repository trust proposal, including adoption of earlier managed branches.
use super::{GithubOperation, SourceClient, Value, bounded, bounded_operation, json};
use ayni_orchestrator_api::v1;

fn managed_branch(branch: &str) -> bool {
    if branch == "setup/certificate-trust" {
        return true;
    }
    let Some(suffix) = branch.strip_prefix("setup/certificate-") else {
        return false;
    };
    let (generation, after) = suffix
        .split_once("-after-")
        .map_or((suffix, None), |(generation, after)| {
            (generation, Some(after))
        });
    generation.len() == 24
        && generation.bytes().all(|c| c.is_ascii_hexdigit())
        && after
            .is_none_or(|number| !number.is_empty() && number.bytes().all(|c| c.is_ascii_digit()))
}
fn select_open(
    pulls: &[Value],
    repository: &str,
    base: &str,
) -> Result<Option<(String, Value)>, &'static str> {
    let mut candidates = pulls.iter().filter(|pull| {
        pull["state"] == "open"
            && pull["merged_at"].is_null()
            && pull["head"]["repo"]["full_name"] == repository
            && pull["base"]["ref"] == base
            && pull["base"]["repo"]["full_name"] == repository
            && pull["head"]["ref"].as_str().is_some_and(managed_branch)
    });
    let Some(pull) = candidates.next() else {
        return Ok(None);
    };
    // Existing duplicate proposals require resolution, never another proposal.
    if candidates.next().is_some() {
        return Err("certificate_trust_proposal_conflict");
    }
    Ok(Some((
        pull["head"]["ref"]
            .as_str()
            .ok_or("certificate_trust_pull_invalid")?
            .into(),
        pull.clone(),
    )))
}
pub(super) fn validate_branch_changes(comparison: &Value) -> Result<(), &'static str> {
    let files = comparison["files"]
        .as_array()
        .ok_or("certificate_trust_branch_conflict")?;
    if files.len() > 2
        || files.iter().any(|file| {
            !matches!(file["filename"].as_str(), Some(".ayni.toml" | ".ayni.lock"))
                || file["previous_filename"]
                    .as_str()
                    .is_some_and(|name| !matches!(name, ".ayni.toml" | ".ayni.lock"))
        })
    {
        return Err("certificate_trust_branch_conflict");
    }
    Ok(())
}
pub(super) fn parents(head: &str, existing: Option<&str>) -> Vec<String> {
    let mut parents = vec![head.to_owned()];
    if let Some(existing) = existing.filter(|existing| *existing != head) {
        parents.push(existing.to_owned());
    }
    parents
}
impl SourceClient {
    pub(crate) async fn find_trust_pull(
        &self,
        github: &v1::GithubRepository,
        base: &str,
        branch: &str,
        token: &str,
    ) -> Result<(String, Option<Value>), &'static str> {
        let mut pulls = Vec::new();
        for page in 1..=10 {
            let page = page.to_string();
            let response = bounded(
                self.client
                    .get(format!(
                        "https://api.github.com/repos/{}/{}/pulls",
                        github.owner, github.name
                    ))
                    .bearer_auth(token)
                    .query(&[
                        ("state", "open"),
                        ("base", base),
                        ("per_page", "100"),
                        ("page", page.as_str()),
                    ]),
            )
            .await?;
            let batch = response
                .as_array()
                .ok_or("certificate_trust_pull_invalid")?;
            pulls.extend(batch.iter().cloned());
            if batch.len() < 100 {
                if let Some((branch, pull)) =
                    select_open(&pulls, &format!("{}/{}", github.owner, github.name), base)?
                {
                    return Ok((branch, Some(pull)));
                }
                // Closed-without-merge proposals can be reopened explicitly. A
                // merged proposal is history; a later required update uses the
                // same branch but necessarily opens a new PR.
                let response = bounded(
                    self.client
                        .get(format!(
                            "https://api.github.com/repos/{}/{}/pulls",
                            github.owner, github.name
                        ))
                        .bearer_auth(token)
                        .query(&[
                            ("state", "closed"),
                            ("head", &format!("{}:{branch}", github.owner)),
                            ("base", base),
                            ("sort", "created"),
                            ("direction", "desc"),
                            ("per_page", "1"),
                        ]),
                )
                .await?;
                let pull = response
                    .as_array()
                    .ok_or("certificate_trust_pull_invalid")?
                    .first()
                    .filter(|pull| {
                        pull["merged_at"].is_null()
                            && pull["head"]["repo"]["full_name"]
                                == format!("{}/{}", github.owner, github.name)
                            && pull["head"]["ref"] == branch
                            && pull["base"]["ref"] == base
                            && pull["base"]["repo"]["full_name"]
                                == format!("{}/{}", github.owner, github.name)
                    })
                    .cloned();
                return Ok((branch.to_owned(), pull));
            }
        }
        Err("certificate_trust_history_limit")
    }
    pub(crate) async fn update_trust_reference(
        &self,
        github: &v1::GithubRepository,
        branch: &str,
        commit: &Value,
        token: &str,
    ) -> Result<(), &'static str> {
        bounded_operation(
            self.client
                .patch(format!(
                    "https://api.github.com/repos/{}/{}/git/refs/heads/{branch}",
                    github.owner, github.name
                ))
                .bearer_auth(token)
                .json(&json!({"sha":commit["sha"],"force":false})),
            GithubOperation::UpdateReference,
        )
        .await
        .map_err(|error| error.reason())?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn pull(branch: &str) -> Value {
        json!({"number":17,"state":"open","merged_at":null,"title":"feat(ayni): update environment signing trust","head":{"ref":branch,"repo":{"full_name":"ayni-oss/sandbox"}},"base":{"ref":"main","repo":{"full_name":"ayni-oss/sandbox"}}})
    }
    #[test]
    fn open_proposals_are_reused_including_legacy_branches_and_reject_duplicates()
    -> Result<(), Box<dyn std::error::Error>> {
        for branch in [
            "setup/certificate-trust",
            "setup/certificate-aaaaaaaaaaaaaaaaaaaaaaaa",
            "setup/certificate-aaaaaaaaaaaaaaaaaaaaaaaa-after-17",
        ] {
            assert_eq!(
                select_open(&[pull(branch)], "ayni-oss/sandbox", "main")?
                    .ok_or("proposal missing")?
                    .0,
                branch
            );
        }
        assert!(
            select_open(
                &[
                    pull("setup/certificate-trust"),
                    pull("setup/certificate-aaaaaaaaaaaaaaaaaaaaaaaa")
                ],
                "ayni-oss/sandbox",
                "main"
            )
            .is_err()
        );
        assert!(
            select_open(
                &[pull("setup/certificate-trust")],
                "other/repository",
                "main"
            )?
            .is_none()
        );
        let mut renamed = pull("setup/certificate-trust");
        renamed["title"] = json!("Owner-edited proposal title");
        assert!(select_open(&[renamed], "ayni-oss/sandbox", "main")?.is_some());
        let mut merged = pull("setup/certificate-trust");
        merged["state"] = json!("closed");
        merged["merged_at"] = json!("2026-10-11T00:00:00Z");
        assert!(select_open(&[merged], "ayni-oss/sandbox", "main")?.is_none());
        Ok(())
    }
    #[test]
    fn branch_updates_preserve_both_histories_and_cannot_discard_unrelated_edits() {
        assert_eq!(
            parents("new-base", Some("proposal")),
            vec!["new-base", "proposal"]
        );
        assert_eq!(parents("base", Some("base")), vec!["base"]);
        assert!(
            validate_branch_changes(
                &json!({"files":[{"filename":".ayni.toml"},{"filename":".ayni.lock"}]})
            )
            .is_ok()
        );
        assert!(validate_branch_changes(&json!({"files":[{"filename":"src/main.rs"}]})).is_err());
        assert!(
            validate_branch_changes(
                &json!({"files":[{"filename":".ayni.toml","previous_filename":"README.md"}]})
            )
            .is_err()
        );
    }
}
