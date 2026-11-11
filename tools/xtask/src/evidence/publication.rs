use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub fn verify(
    baseline: &Value,
    current: &Value,
    creates: u64,
    pulls: usize,
    identity: Option<&Value>,
) -> Result<Value> {
    let before = baseline["create_calls"]
        .as_u64()
        .context("baseline create count")?;
    let after = current["create_calls"]
        .as_u64()
        .context("current create count")?;
    ensure!(
        after.checked_sub(before) == Some(creates),
        "unexpected publication create request delta"
    );
    let previous = baseline["pulls"].as_array().context("baseline pulls")?;
    let current_pulls = current["pulls"].as_array().context("current pulls")?;
    ensure!(
        current_pulls.len().checked_sub(previous.len()) == Some(pulls),
        "unexpected retained publication delta"
    );
    let retained: BTreeMap<_, _> = current_pulls
        .iter()
        .map(|pull| {
            pull["number"]
                .as_u64()
                .map(|number| (number, pull))
                .context("invalid pull number")
        })
        .collect::<Result<_>>()?;
    ensure!(
        retained.len() == current_pulls.len(),
        "duplicate retained publication number"
    );
    for pull in previous {
        ensure!(
            retained
                .get(&pull["number"].as_u64().context("baseline pull number")?)
                .copied()
                == Some(pull),
            "previously retained publication changed"
        );
    }
    let references = current["references"]
        .as_array()
        .context("current references")?;
    ensure!(
        baseline["references"]
            .as_array()
            .context("baseline references")?
            .iter()
            .all(|r| references.contains(r)),
        "previously retained Git reference changed"
    );
    if let Some(identity) = identity {
        let number = identity["pr_number"]
            .as_u64()
            .context("selected pull number")?;
        let pull = retained
            .get(&number)
            .context("current publication identity missing")?;
        ensure!(
            pull["head"]["ref"] == identity["branch"],
            "publication branch differs"
        );
        ensure!(
            !previous.iter().any(|old| old["number"] == number),
            "current publication reused retained identity"
        );
        ensure!(
            pull["head"]["sha"] == identity["commit_sha"]
                && pull["draft"] == true
                && pull["state"] == "open",
            "current publication differs from selected draft"
        );
        let reference = format!(
            "refs/heads/{} {}",
            identity["branch"].as_str().context("branch")?,
            identity["commit_sha"].as_str().context("commit")?
        );
        ensure!(
            references.contains(&Value::String(reference)),
            "current publication Git reference missing"
        );
    }
    Ok(
        json!({"create_requests":creates,"retained_prs":pulls,"baseline_create_requests":before,"baseline_retained_prs":previous.len(),"total_create_requests":after,"total_retained_prs":current_pulls.len()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> (Value, Value, Value) {
        let old = json!({"number":3,"head":{"ref":"old","sha":"abc"},"draft":true,"state":"open"});
        let new =
            json!({"number":4,"head":{"ref":"selected","sha":"def"},"draft":true,"state":"open"});
        (
            json!({"create_calls":7,"pulls":[old],"references":["refs/heads/old abc"]}),
            json!({"create_calls":8,"pulls":[old,new],"references":["refs/heads/old abc","refs/heads/selected def"]}),
            json!({"pr_number":4,"branch":"selected","commit_sha":"def"}),
        )
    }

    #[test]
    fn counts_only_this_run_and_preserves_prior_history() -> Result<()> {
        let (before, after, identity) = fixtures();
        let result = verify(&before, &after, 1, 1, Some(&identity))?;
        assert_eq!(result["total_create_requests"], 8);
        assert_eq!(result["baseline_retained_prs"], 1);
        verify(
            &json!({"create_calls":0,"pulls":[],"references":[]}),
            &after,
            8,
            2,
            None,
        )?;
        Ok(())
    }

    #[test]
    fn rejects_duplicate_effects_changed_history_and_reused_identity() {
        let (before, after, identity) = fixtures();
        for (pointer, value) in [
            ("/create_calls", json!(9)),
            ("/pulls/0/state", json!("closed")),
            ("/references/0", json!("refs/heads/old changed")),
            ("/pulls/1/head/sha", json!("wrong")),
            ("/pulls/1/number", json!(3)),
        ] {
            let mut changed = after.clone();
            if let Some(field) = changed.pointer_mut(pointer) {
                *field = value;
            }
            assert!(
                verify(&before, &changed, 1, 1, Some(&identity)).is_err(),
                "accepted changed {pointer}"
            );
        }
        assert!(
            verify(
                &before,
                &after,
                1,
                1,
                Some(&json!({"pr_number":3,"branch":"old","commit_sha":"abc"}))
            )
            .is_err()
        );
    }
}
