#!/usr/bin/env python3
"""Aggregate executed gates while preserving explicitly unmeasured acceptance scope."""
import hashlib
import json
from pathlib import Path
import sys

REPLAY = "operational-concurrency: four-active=pass two-queued=pass isolated-pod-identities=pass dependency-blockers=pass cyclic-rejection=pass cancellation=pass exact-replay=pass"


def build_report(root):
    root = Path(root)
    for profile in ("m4", "m6"):
        report = json.loads((root / profile / "acceptance.json").read_text())
        if report["result"] != "passed":
            raise ValueError(f"{profile} cumulative gates failed")
    evidence = json.loads((root / "concurrency-terminal.json").read_text())
    for field in ("requests", "receipts", "active", "cancellation_requests", "cancellations", "terminal"):
        if len(evidence[field]) != 6:
            raise ValueError(f"incomplete concurrency evidence: {field}")
    if not evidence["blocked_dependency"]:
        raise ValueError("missing dependency admission evidence")
    for owner in ("control-plane", "task-supervisor", "workflow-postgres"):
        if (root / f"concurrency-replay-{owner}.txt").read_text().strip() != REPLAY:
            raise ValueError(f"missing exact concurrent replay: {owner}")
    for required in ("concurrency-adoption.txt", "operational-readiness.txt", "conversation-diagnostic.txt", "postgres-integration.txt"):
        if not (root / required).read_text().strip():
            raise ValueError(f"missing operational evidence: {required}")
    if "diagnostic passed" not in (root / "conversation-diagnostic.txt").read_text():
        raise ValueError("conversation diagnostic did not pass")
    self_development = root / "self-development/self-development.json"
    own_source_passed = False
    if self_development.exists():
        own_source = json.loads(self_development.read_text())
        if own_source.get("result") != "passed" or own_source.get("quality", {}).get("state") != "passed":
            raise ValueError("own-source managed self-development did not pass")
        if own_source.get("actual_source_repository") != "https://github.com/ayni-oss/orchestrator.git" or own_source.get("selected_source_paths") != ["README.md"]:
            raise ValueError("self-development evidence used another source or coding scope")
        if not own_source.get("publication", {}).get("protocol_fixture") or own_source.get("live_provider") or own_source.get("live_github"):
            raise ValueError("self-development fixture boundary changed")
        own_source_passed = True
    report = {
        "schema": "build.ayni.orchestrator.operational-acceptance.v1",
        "result": "passed",
        "milestone_complete": False,
        "completed_gates": ["cumulative-M4-failure-matrix", "cumulative-independent-task-M6-failure-matrix",
                            "four-isolated-active-attempts-and-two-queued", "cyclic-plan-rejection",
                            "unselected-dependencies-block-preparation", "concurrent-supervisor-adoption",
                            "six-task-cancellation-and-exact-replay-after-owner-restarts",
                            "deployed-readiness-and-conversation-diagnostic", "disposable-PostgreSQL-integration"],
        "unmeasured_gates": ["deployed-accepted-multi-upstream-commit-integration-and-recovery",
                             "deployed-worker-question-and-model-proposal-recovery",
                             "deployed-outcome-assessment-and-MCP-external-effect-failure-matrix",
                             "live-additional-provider-contract-evidence",
                             "thirty-day-retention-and-production-capacity-envelope",
                             "orchestrator-self-development-through-managed-quality-and-draft-publication"],
        "fixture_boundaries": {"coding": "explicit deterministic fixture", "quality": "actual managed Ayni runtime",
                               "publication": "real Git transport and GitHub protocol fixture", "live_provider": False},
        "artifacts_sha256": {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest()
                             for p in sorted(root.rglob("*")) if p.is_file() and p.name not in ("acceptance.json", "outcome.json")},
    }
    if own_source_passed:
        gate = "orchestrator-self-development-through-managed-quality-and-draft-publication"
        report["unmeasured_gates"].remove(gate)
        report["completed_gates"].append(gate)
    return report


def main():
    root = Path(sys.argv[1])
    destination = root / "acceptance.json"
    if destination.exists():
        raise ValueError("refusing to replace existing acceptance evidence")
    destination.write_text(json.dumps(build_report(root), indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
