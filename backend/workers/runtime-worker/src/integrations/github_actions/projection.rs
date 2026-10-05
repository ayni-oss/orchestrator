use super::*;
pub(super) async fn project(
    repository: &str,
    manifest: &Manifest,
    artifacts: &BTreeMap<String, Vec<u8>>,
) -> Result<(), &'static str> {
    let number = manifest.run.pull_request;
    // Re-read immediately before each projection. Stale runs never replace a
    // newer head's summary; their immutable evidence remains in Actions.
    let current = api(&format!("repos/{repository}/pulls/{number}"), None).await?;
    if current["head"]["sha"] != manifest.run.head {
        return Ok(());
    }
    let link = format!(
        "https://github.com/{repository}/actions/runs/{}",
        manifest.run.run_id
    );
    let outcome = serde_json::to_value(manifest.outcome).map_err(|_| "outcome_invalid")?;
    let title = outcome.as_str().ok_or("outcome_invalid")?;
    let external = format!(
        "assessment:{}:{}",
        manifest.run.run_id, manifest.run.attempt
    );
    let checks = api(
        &format!(
            "repos/{repository}/commits/{}/check-runs?per_page=100",
            manifest.run.head
        ),
        None,
    )
    .await?;
    let existing = checks["check_runs"]
        .as_array()
        .filter(|c| c.len() < 100)
        .ok_or("checks_invalid")?
        .iter()
        .find(|c| c["external_id"] == external && c["app"]["slug"] == "github-actions");
    let body = summary(manifest, artifacts, &link);
    let check = json!({"name":assessment::CHECK_NAME,"head_sha":manifest.run.head,"external_id":external,"status":"completed",
        "conclusion":if manifest.outcome == Outcome::Succeeded {"success"} else if matches!(manifest.outcome, Outcome::Stale | Outcome::Cancelled) {"cancelled"} else {"failure"},
        "details_url":link,"output":{"title":title,"summary":body}});
    // A response can be lost: discover by immutable external_id before creating.
    if existing.is_none() {
        api(&format!("repos/{repository}/check-runs"), Some(&check)).await?;
    }
    let marker = "<!-- orchestrator-assessment:v1 -->";
    let end_marker = "<!-- /orchestrator-assessment:v1 -->";
    let block = format!("{marker}\n{body}\n{end_marker}");
    let comments = api(
        &format!("repos/{repository}/issues/{number}/comments?per_page=100"),
        None,
    )
    .await?;
    let comments = comments
        .as_array()
        .filter(|c| c.len() < 100)
        .ok_or("comment_limit")?;
    let owned: Vec<_> = comments
        .iter()
        .filter(|c| {
            c["user"]["login"] == "github-actions[bot]"
                && c["body"].as_str().is_some_and(|body| body.contains(marker))
        })
        .collect();
    if owned.len() > 1 {
        return Err("comment_identity_conflict");
    }
    let current = api(&format!("repos/{repository}/pulls/{number}"), None).await?;
    if current["head"]["sha"] != manifest.run.head {
        return Ok(());
    }
    if let Some(comment) = owned.first() {
        let old = comment["body"].as_str().ok_or("comment_invalid")?;
        let updated = replace_summary_block(old, &block)?;
        let id = comment["id"].as_u64().ok_or("comment_invalid")?;
        api(
            &format!("repos/{repository}/issues/comments/{id}"),
            Some(&json!({"body":updated})),
        )
        .await?;
    } else {
        api(
            &format!("repos/{repository}/issues/{number}/comments"),
            Some(&json!({"body":block})),
        )
        .await?;
    }
    Ok(())
}

pub(super) fn summary(
    manifest: &Manifest,
    artifacts: &BTreeMap<String, Vec<u8>>,
    link: &str,
) -> String {
    // Raw tool output, arbitrary URLs and paths never enter the Markdown. Only
    // bounded counts and enum-like signal labels are projected.
    let mut body = format!(
        "### Ayni assessment: {:?}\n\nHead: `{}`\n\n",
        manifest.outcome, manifest.run.head
    );
    if let Some(signals) = artifacts
        .get("signals.json")
        .and_then(|b| serde_json::from_slice::<Value>(b).ok())
    {
        let count = |section: &str, field: &str| signals[section][field].as_u64().unwrap_or(0);
        let _ = write!(
            body,
            "Targets: {} completed, {} skipped. Warnings: {}. Failures: {}.\n\n",
            count("completion", "completed_targets"),
            count("completion", "skipped_targets"),
            count("aggregate", "warning_offenders"),
            count("aggregate", "failing_offenders")
        );
        if let Some(rows) = signals["rows"].as_array() {
            body.push_str("| Signal | Result |\n| --- | --- |\n");
            for row in rows.iter().take(32) {
                let kind = row["kind"]
                    .as_str()
                    .filter(|v| {
                        matches!(
                            *v,
                            "test" | "coverage" | "size" | "complexity" | "deps" | "mutation"
                        )
                    })
                    .unwrap_or("other");
                let _ = writeln!(
                    body,
                    "| {kind} | {} |",
                    if row["pass"] == true { "pass" } else { "fail" }
                );
            }
        }
    }
    let _ = write!(
        body,
        "\nSummary is bounded; detailed findings are retained in [workflow evidence]({link}).\n"
    );
    body
}

pub(super) fn replace_summary_block(old: &str, block: &str) -> Result<String, &'static str> {
    let marker = "<!-- orchestrator-assessment:v1 -->";
    let end_marker = "<!-- /orchestrator-assessment:v1 -->";
    if old.matches(marker).count() != 1 || old.matches(end_marker).count() != 1 {
        return Err("comment_marker_damaged");
    }
    let start = old.find(marker).ok_or("comment_invalid")?;
    let end = old[start..]
        .find(end_marker)
        .map(|e| start + e + end_marker.len())
        .ok_or("comment_marker_damaged")?;
    let updated = format!("{}{}{}", &old[..start], block, &old[end..]);
    if updated.len() > 60_000 {
        return Err("comment_oversized");
    }
    Ok(updated)
}
