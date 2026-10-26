#!/usr/bin/env python3
"""Compile completed recovery evidence without claiming missing milestone prerequisites."""
import hashlib
import json
from pathlib import Path
import sys

SCENARIOS = ("queued-cancel", "active-cancel", "pod-loss")
OWNERS = ("control-plane", "task-supervisor", "conversation-supervisor", "jetstream", "postgres", "redeploy")
REPLAY = "recovery-fault: exact-replay=pass saved-work=pass parent-evidence=pass terminal-fence=pass attention=pass"
UNMET = ["M5 dependency scheduling and frozen upstream selections", "worker-originated structured questions and answer delivery", "model-generated recovery proposals"]


def build_report(root):
    root = Path(root)
    required = ["quality/source-revision.txt", "quality/source-manifest.json", "quality/image-identities.json",
                "confirmation-restart.txt", "supervisor-adoption.txt", "late-worker-exit.txt", "late-worker-fence.txt",
                "continued-pod-loss.txt", "cleanup.txt", "redeploy.txt"]
    for name in required:
        if not (root / name).read_text().strip():
            raise ValueError(f"missing evidence: {name}")
    if json.loads((root / "quality/outcome.json").read_text())["result"] != "passed":
        raise ValueError("cumulative quality profile did not pass")
    browser = json.loads((root / "quality/recovery-console/outcome.json").read_text())
    if browser["result"] != "passed" or not {"two-tab-confirmation", "pending-origin-after-navigation", "attention-resolution"}.issubset(browser["cases"]):
        raise ValueError("complete browser recovery evidence required")
    if (root / "late-worker-exit.txt").read_text().strip() != "1":
        raise ValueError("continued worker did not reject late output")
    if "late-output-rejected=pass" not in (root / "late-worker-fence.txt").read_text():
        raise ValueError("continued worker fence was not verified")
    inventories = [(root / f"retained-{stage}.txt").read_bytes() for stage in ("before", "stopped", "after")]
    if not inventories[0] or not inventories[0] == inventories[1] == inventories[2]:
        raise ValueError("retained identities changed during cleanup/redeployment")
    for scenario in SCENARIOS:
        terminal = json.loads((root / f"{scenario}-terminal.json").read_text())
        for field in ("parent_request", "parent", "proposal", "confirmation_request", "confirmation", "terminal", "feedback"):
            if not terminal[field]:
                raise ValueError(f"missing {scenario} {field}")
        if scenario != "pod-loss" and (not terminal["cancellation_request"] or not terminal["cancellation"]):
            raise ValueError("cancelled continuation lacks its exact receipt")
        for owner in OWNERS:
            if (root / f"{scenario}-replay-{owner}.txt").read_text().strip() != REPLAY:
                raise ValueError(f"unverified recovery replay: {scenario} {owner}")
    artifacts = {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest()
                 for p in sorted(root.rglob("*")) if p.is_file() and p.relative_to(root) not in (Path("acceptance.json"), Path("outcome.json"))}
    return {
        "schema": "build.ayni.orchestrator.task-recovery-acceptance.v1",
        "result": "passed", "profile": "independent-task-recovery", "milestone_complete": False,
        "completed_gates": ["cumulative-managed-quality", "conversation-consent-response-loss-navigation",
                            "different-key-confirmation-convergence", "queued-recovery-authority-restart-and-cancellation",
                            "active-recovery-supervisor-adoption", "active-recovery-cancellation-and-late-report-fence",
                            "continued-worker-pod-loss", "saved-work-and-parent-evidence", "attention-resolution-and-new-interruption",
                            "all-owner-restart-replay", "retained-cleanup-and-redeployment"],
        "unmet_prerequisites": UNMET,
        "fixture_boundaries": {"coding": "explicit deterministic worker fixture", "quality": "actual managed Ayni worker runtime",
                               "live_provider_recovery": False, "live_github_recovery": False},
        "artifacts_sha256": artifacts,
    }


def main():
    root = Path(sys.argv[1])
    destination = root / "acceptance.json"
    if destination.exists():
        raise ValueError("refusing to replace existing acceptance evidence")
    report = build_report(root)
    destination.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
