use super::{json, manifest, read};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::path::Path;

const SCENARIOS: &[&str] = &["queued-cancel", "active-cancel", "pod-loss"];
const OWNERS: &[&str] = &[
    "control-plane",
    "task-supervisor",
    "conversation-supervisor",
    "jetstream",
    "postgres",
    "redeploy",
];
const REPLAY: &str = "recovery-fault: exact-replay=pass saved-work=pass parent-evidence=pass terminal-fence=pass attention=pass";

pub fn build(root: &Path) -> Result<Value> {
    for name in [
        "quality/source-revision.txt",
        "quality/source-manifest.json",
        "quality/image-identities.json",
        "confirmation-restart.txt",
        "supervisor-adoption.txt",
        "late-worker-exit.txt",
        "late-worker-fence.txt",
        "continued-pod-loss.txt",
        "cleanup.txt",
        "redeploy.txt",
    ] {
        read(root, name)?;
    }
    ensure!(
        json(root, "quality/outcome.json")?["result"] == "passed",
        "cumulative quality did not pass"
    );
    let browser = json(root, "quality/recovery-console/outcome.json")?;
    ensure!(
        browser["result"] == "passed"
            && [
                "two-tab-confirmation",
                "pending-origin-after-navigation",
                "attention-resolution"
            ]
            .iter()
            .all(|case| browser["cases"]
                .as_array()
                .is_some_and(|cases| cases.contains(&json!(case)))),
        "complete browser recovery evidence required"
    );
    ensure!(
        read(root, "late-worker-exit.txt")?.trim() == "1"
            && read(root, "late-worker-fence.txt")?.contains("late-output-rejected=pass"),
        "late worker fence not verified"
    );
    let retained = read(root, "retained-before.txt")?;
    ensure!(
        retained == read(root, "retained-stopped.txt")?
            && retained == read(root, "retained-after.txt")?,
        "retained identities changed during redeployment"
    );
    for scenario in SCENARIOS {
        let terminal = json(root, &format!("{scenario}-terminal.json"))?;
        for field in [
            "parent_request",
            "parent",
            "proposal",
            "confirmation_request",
            "confirmation",
            "terminal",
            "feedback",
        ] {
            ensure!(
                terminal[field].as_array().is_some_and(|a| !a.is_empty()),
                "missing {scenario} {field}"
            );
        }
        if *scenario != "pod-loss" {
            for field in ["cancellation_request", "cancellation"] {
                ensure!(
                    terminal[field].as_array().is_some_and(|a| !a.is_empty()),
                    "missing cancellation receipt"
                );
            }
        }
        for owner in OWNERS {
            ensure!(
                read(root, &format!("{scenario}-replay-{owner}.txt"))?.trim() == REPLAY,
                "unverified replay {scenario}/{owner}"
            );
        }
    }
    Ok(
        json!({"schema":"build.ayni.orchestrator.task-recovery-acceptance.v1","result":"passed","profile":"independent-task-recovery","milestone_complete":false,
        "completed_gates":["cumulative-managed-quality","conversation-consent-response-loss-navigation","different-key-confirmation-convergence","queued-recovery-authority-restart-and-cancellation","active-recovery-supervisor-adoption","active-recovery-cancellation-and-late-report-fence","continued-worker-pod-loss","saved-work-and-parent-evidence","attention-resolution-and-new-interruption","all-owner-restart-replay","retained-cleanup-and-redeployment"],
        "unmet_prerequisites":["M5 dependency scheduling and frozen upstream selections","worker-originated structured questions and answer delivery","model-generated recovery proposals"],
        "fixture_boundaries":{"coding":"explicit deterministic worker fixture","quality":"actual managed Ayni worker runtime","live_provider_recovery":false,"live_github_recovery":false},"artifacts_sha256":manifest(root)?}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture(root: &Path) -> Result<()> {
        fs::create_dir_all(root.join("quality/recovery-console"))?;
        for name in [
            "quality/source-revision.txt",
            "quality/source-manifest.json",
            "quality/image-identities.json",
            "confirmation-restart.txt",
            "supervisor-adoption.txt",
            "continued-pod-loss.txt",
            "cleanup.txt",
            "redeploy.txt",
        ] {
            fs::write(root.join(name), "retained evidence")?;
        }
        fs::write(root.join("quality/outcome.json"), r#"{"result":"passed"}"#)?;
        fs::write(
            root.join("quality/recovery-console/outcome.json"),
            r#"{"result":"passed","cases":["two-tab-confirmation","pending-origin-after-navigation","attention-resolution"]}"#,
        )?;
        fs::write(root.join("late-worker-exit.txt"), "1\n")?;
        fs::write(
            root.join("late-worker-fence.txt"),
            "late-output-rejected=pass",
        )?;
        for stage in ["before", "stopped", "after"] {
            fs::write(root.join(format!("retained-{stage}.txt")), "same")?;
        }
        for scenario in SCENARIOS {
            let mut value = json!({});
            for field in [
                "parent_request",
                "parent",
                "proposal",
                "confirmation_request",
                "confirmation",
                "terminal",
                "feedback",
                "cancellation_request",
                "cancellation",
            ] {
                value[field] = json!([1]);
            }
            fs::write(
                root.join(format!("{scenario}-terminal.json")),
                serde_json::to_vec(&value)?,
            )?;
            for owner in OWNERS {
                fs::write(root.join(format!("{scenario}-replay-{owner}.txt")), REPLAY)?;
            }
        }
        Ok(())
    }

    #[test]
    fn passing_fixture_cannot_claim_full_milestone_closure() -> Result<()> {
        let root = tempfile::tempdir()?;
        fixture(root.path())?;
        let result = build(root.path())?;
        assert_eq!(result["milestone_complete"], false);
        assert_eq!(
            result["fixture_boundaries"]["live_provider_recovery"],
            false
        );
        assert!(result["artifacts_sha256"]["quality/recovery-console/outcome.json"].is_string());
        Ok(())
    }

    #[test]
    fn each_missing_replay_blocks_aggregate_success() -> Result<()> {
        for scenario in SCENARIOS {
            for owner in OWNERS {
                let root = tempfile::tempdir()?;
                fixture(root.path())?;
                fs::remove_file(root.path().join(format!("{scenario}-replay-{owner}.txt")))?;
                assert!(build(root.path()).is_err());
            }
        }
        Ok(())
    }

    #[test]
    fn changed_storage_successful_late_worker_or_failed_browser_cannot_pass() -> Result<()> {
        for (name, value) in [
            ("retained-after.txt", "replaced volume"),
            ("late-worker-exit.txt", "0"),
            (
                "quality/recovery-console/outcome.json",
                r#"{"result":"failed","cases":[]}"#,
            ),
        ] {
            let root = tempfile::tempdir()?;
            fixture(root.path())?;
            fs::write(root.path().join(name), value)?;
            assert!(build(root.path()).is_err());
        }
        Ok(())
    }
}
