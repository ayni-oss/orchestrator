#!/usr/bin/env python3
"""Validate cumulative deployed evidence without promoting fixtures to live claims."""
import hashlib
import json
from pathlib import Path
import re
import sys

SCHEMA = "build.ayni.orchestrator.operational-acceptance.v1"
REPLAY = "operational-concurrency: four-active=pass two-queued=pass isolated-pod-identities=pass dependency-blockers=pass cyclic-rejection=pass cancellation=pass exact-replay=pass"
DEPENDENCY_GATES = {"two-accepted-upstream-draft-commits", "trusted-integrated-source-ancestry", "frozen-dependent-inputs", "managed-dependent-quality-failure", "confirmed-dependent-repair", "dependency-recovery-owner-restart-exact-replay"}
ASSESSMENT_GATES = {"console-explicit-bounded-consent", "consent-response-loss-reload-exact-retry", "deployed-progress-spans-maintenance-without-model", "accepted-worker-outcome-triggers-assessment", "changed-outcome-rejects-stale-assessment", "originating-reviewable-proposal-with-provider-provenance", "proposal-grants-no-execution", "bounded-assessment-count", "explicit-revocation", "conversation-supervisor-restart-no-resend", "exact-proposal-replay", "retained-revocation"}
MCP_GATES = {"scoped-tools-and-exact-consent", "retained-successful-tool-output", "external-effect-response-loss", "in-flight-external-effect-cancellation", "pod-replacement-exact-replay"}
WORKER_RECOVERY_GATES = {"worker-origin-question-and-proposal", "unanswered-question-blocks-confirmation", "answer-and-exact-confirmation", "answered-model-recovery-managed-quality", "worker-recovery-restart-no-resend"}
M6_GATES = {"cumulative-managed-quality", "conversation-consent-response-loss-navigation", "different-key-confirmation-convergence", "queued-recovery-authority-restart-and-cancellation", "active-recovery-supervisor-adoption", "active-recovery-cancellation-and-late-report-fence", "continued-worker-pod-loss", "saved-work-and-parent-evidence", "attention-resolution-and-new-interruption", "all-owner-restart-replay", "retained-cleanup-and-redeployment"}
M4_CASES = {"retained-fixture-baseline", "frozen-harness-source", "approval-and-preparation-restart", "coding-pod-loss", "late-coding-report-fenced", "managed-quality-pass-and-failure", "browser-publication-retry-and-cancel", "publisher-loss-before-create", "publisher-loss-after-remote-create", "uncertain-without-visible-pr", "existing-branch-conflict", "deployed-credential-and-source-isolation", "cleanup-and-redeployment"}

M4_CASES |= {f"restart-{owner}" for owner in ("control-plane", "task-publisher", "publication-fixture", "task-supervisor", "scripted-executor", "workflow-jetstream", "workflow-postgres")}

def read(root, name):
    target = root / name
    if not target.is_file() or target.is_symlink():
        raise ValueError(f"missing regular evidence file: {name}")
    value = target.read_text()
    if not value.strip():
        raise ValueError(f"empty evidence: {name}")
    return value


def record(root, name, schema, gates=()):
    value = json.loads(read(root, name))
    if value.get("schema") != schema or value.get("result") != "passed":
        raise ValueError(f"invalid or failed profile: {name}")
    if not set(gates).issubset(value.get("gates", value.get("completed_gates", []))):
        raise ValueError(f"incomplete profile gates: {name}")
    return value


def verify_manifest(root, manifest, required=()):
    root = Path(root).resolve()
    if not isinstance(manifest, dict) or not manifest:
        raise ValueError("missing artifact fingerprints")
    if not set(required).issubset(manifest):
        raise ValueError("required retained artifacts lack fingerprints")
    for name, expected in manifest.items():
        path = Path(name)
        if path.is_absolute() or ".." in path.parts or not path.parts:
            raise ValueError("artifact fingerprint escapes evidence root")
        target = root / path
        if any(p.is_symlink() for p in (target, *target.parents)) or not target.is_file():
            raise ValueError(f"artifact is not a retained regular file: {name}")
        if not isinstance(expected, str) or not re.fullmatch(r"[a-f0-9]{64}", expected):
            raise ValueError(f"invalid artifact fingerprint: {name}")
        if hashlib.sha256(target.read_bytes()).hexdigest() != expected:
            raise ValueError(f"artifact changed: {name}")


def retained_bytes(value):
    return isinstance(value, list) and bool(value) and all(isinstance(n, int) and not isinstance(n, bool) and 0 <= n < 256 for n in value)


def concurrency(root):
    before = json.loads(read(root, "concurrency-active.json"))
    adopted = json.loads(read(root, "concurrency-adopted.json"))
    evidence = json.loads(read(root, "concurrency-terminal.json"))
    for field in ("requests", "receipts", "active", "cancellation_requests", "cancellations", "terminal"):
        values = evidence.get(field, [])
        if len(values) != 6 or not all(retained_bytes(v) for v in values) or len({bytes(v) for v in values}) != 6:
            raise ValueError(f"incomplete or duplicated concurrency evidence: {field}")
    if not retained_bytes(evidence.get("blocked_dependency")):
        raise ValueError("missing dependency admission evidence")
    for field in ("scope", "requests", "receipts", "blocked_dependency", "active"):
        if not before.get(field) or before[field] != adopted.get(field) or before[field] != evidence.get(field):
            raise ValueError(f"concurrency authority changed during adoption: {field}")
    pods = json.loads(read(root, "concurrency-pods.json"))["items"]
    active = [bytes(v) for v in before["active"]]
    matched = {p["metadata"]["uid"] for p in pods if any(p["metadata"]["uid"].encode() in v for v in active)}
    if len(matched) != 4 or sum(any(uid.encode() in value for uid in matched) for value in active) != 4:
        raise ValueError("missing four distinct deployed active Pod identities and two queued attempts")
    for owner in ("control-plane", "task-supervisor", "workflow-postgres"):
        if read(root, f"concurrency-replay-{owner}.txt").strip() != REPLAY:
            raise ValueError(f"missing exact concurrent replay: {owner}")


def mcp_replay(root, provider):
    initial, replay = [json.loads(read(root, f"{stage}.json")) for stage in ("initial", "replay")]
    for value in (initial, replay):
        if value.get("outcome") != "passed" or value.get("provider") != provider or value.get("profile") != "deployed-provider-mcp-worker":
            raise ValueError("invalid MCP driver result")
        success = value.get("success_result", {})
        call = success.get("provider_call", {})
        tools = call.get("toolInvocations", [])
        if call.get("state") != "completed" or call.get("provider") != provider or str(call.get("totalTokens")) != "40" or len(tools) != 2 or {t.get("name") for t in tools} != {"mcp_fixture_read", "mcp_fixture_write"} or any(t.get("state") != "completed" for t in tools) or success.get("failure_reason") != "" or not success.get("response_content"):
            raise ValueError("missing successful MCP tool and usage evidence")
        effects = value.get("external_effect_failures", [])
        expected = {"fixture:mcp-lost": ("provider_tool_uncertain", "mcp_fixture_lost", "failed", "mcpLostEffects"), "fixture:mcp-cancel": ("cancelled", "mcp_fixture_cancel", "cancelled", "mcpCancelledEffects")}
        if len(effects) != 2 or {e.get("mode") for e in effects} != set(expected):
            raise ValueError("missing once-only MCP external-effect failures")
        for effect in effects:
            reason, tool, state, counter = expected[effect["mode"]]
            result = effect.get("result", {})
            call = result.get("provider_call", {})
            invocations = call.get("toolInvocations", [])
            if effect.get("external_effects") != 1 or effect.get("effect_counter") != counter or result.get("failure_reason") != reason or result.get("response_content") != "" or call.get("provider") != provider or call.get("state") != state or len(invocations) != 1 or invocations[0].get("name") != tool or invocations[0].get("state") != "uncertain":
                raise ValueError("MCP external effect lost its retained uncertainty")
            if not effect.get("command_id") or effect["command_id"] != result.get("command_id") or not effect.get("command_digest") or effect["command_digest"] != result.get("command_digest"):
                raise ValueError("MCP failure result changed its command identity")
        failures = value.get("provider_failures", [])
        if provider == "anthropic":
            if len(failures) != 2 or failures[0].get("provider_call", {}).get("state") != "uncertain" or failures[1].get("provider_call", {}).get("state") not in ("uncertain", "cancelled") or any(f.get("response_content") != "" or f.get("provider_call", {}).get("provider") != provider for f in failures):
                raise ValueError("missing retained Anthropic uncertainty and cancellation")
        elif failures:
            raise ValueError("unexpected provider failure profile")
    for name in ("success_result", "external_effect_failures", "provider_failures", "command_id", "command_digest"):
        if name not in initial or initial[name] != replay.get(name):
            raise ValueError(f"MCP replacement changed retained {name}")
    stats = replay.get("final_stats", {})
    if initial.get("replay") is not False or replay.get("replay") is not True or stats.get("calls") != 0 or stats.get("mcpReads") != 0 or any(stats.get(n) != 1 for n in ("mcpEffects", "mcpLostEffects", "mcpCancelledEffects")):
        raise ValueError("MCP replay repeated a provider call or external effect")
    pods = [json.loads(read(root, f"{stage}-pod.json")) for stage in ("initial", "replay")]
    pvcs = [json.loads(read(root, f"{stage}-pvc.json")) for stage in ("initial", "replay")]
    if not pods[0]["metadata"].get("uid") or pods[0]["metadata"]["uid"] == pods[1]["metadata"].get("uid") or not pvcs[0]["metadata"].get("uid") or pvcs[0]["metadata"]["uid"] != pvcs[1]["metadata"].get("uid"):
        raise ValueError("MCP replacement did not retain the same authority PVC")
    images = [{c["name"]: c.get("imageID", "") for c in pod.get("status", {}).get("containerStatuses", [])} for pod in pods]
    if not images[0] or not all(images[0].values()) or images[0] != images[1]:
        raise ValueError("MCP replay used different or missing runtime image identities")


def json_proof(root, name):
    return json.loads(read(root, name))


def dependency_proofs(root, report):
    owners = ("control-plane", "task-supervisor", "workflow-postgres")
    required = {"dependent-quality-failed.json", "dependent-repaired.json", "dependent-repaired-draft.json", "repair-confirmation.json", "repair-proposal.json", "dependent-request.json", "upstream-0-draft.json", "upstream-1-draft.json"}
    required |= {f"{kind}-replay-{owner}.json" for owner in owners for kind in ("confirmation", "preparation", "child", "parent")}
    verify_manifest(root, report.get("artifacts_sha256"), required)
    parent = json_proof(root, "dependent-quality-failed.json")
    child = json_proof(root, "dependent-repaired.json")
    draft = json_proof(root, "dependent-repaired-draft.json")
    confirmation = json_proof(root, "repair-confirmation.json")["response"]
    inputs = json_proof(root, "dependent-request.json")["inputs"]
    upstreams = [json_proof(root, f"upstream-{n}-draft.json") for n in range(2)]
    selected = [{"taskId": u["taskId"], "attemptId": u["attemptId"], "commitSha": u["publication"]["commitSha"]} for u in upstreams]
    if len({u["commitSha"] for u in selected}) != 2 or selected != inputs.get("upstreamSelections") or selected != report.get("upstream_selections"):
        raise ValueError("dependent inputs lost their two accepted upstream identities")
    if any(u.get("publication", {}).get("state") != "draft" or u.get("quality", {}).get("state") != "passed" for u in upstreams):
        raise ValueError("upstream commits lack accepted managed-quality drafts")
    source = report.get("integrated_source_commit")
    if not source or inputs.get("sourceCommit") != source or any(a.get("sourceCommit") != source for a in (parent, child, draft)):
        raise ValueError("dependent source changed through confirmed repair")
    if parent.get("quality", {}).get("state") != "failed" or child.get("quality", {}).get("state") != "passed" or draft.get("publication", {}).get("state") != "draft" or not parent.get("attemptId") or child.get("parentAttemptId") != parent["attemptId"] or child.get("savedInputPatch") != parent.get("patch") or not child.get("savedInputPatch") or confirmation.get("confirmation", {}).get("attemptId") != child.get("attemptId"):
        raise ValueError("dependent repair lost parent, saved-work or confirmation authority")
    proposal = json_proof(root, "repair-proposal.json")["request"]["proposal"]
    allowances = {"maxSeconds": proposal.get("additionalSeconds"), "maxTokens": proposal.get("additionalTokens")}
    if any(not value or child.get(name) != value for name, value in allowances.items()):
        raise ValueError("dependent repair allowances differ from the reviewed proposal")
    child_inputs = {**inputs, **allowances}
    retained_preparation = json_proof(root, "preparation-replay-control-plane.json")["response"]
    preparation = retained_preparation.get("preparation", {})
    if preparation.get("inputs") != child_inputs or preparation.get("attemptId") != child.get("attemptId"):
        raise ValueError("continued preparation lost frozen inputs or confirmed allowances")
    for owner in owners:
        if json_proof(root, f"confirmation-replay-{owner}.json")["response"] != confirmation or json_proof(root, f"parent-replay-{owner}.json")["response"].get("attempt") != parent or json_proof(root, f"child-replay-{owner}.json")["response"].get("attempt") != draft or json_proof(root, f"preparation-replay-{owner}.json")["response"] != retained_preparation:
            raise ValueError("dependency authority changed after owner restart")


def worker_proofs(root, report):
    required = {"parent.json", "worker-records.json", "unanswered-confirmation.json", "answer.json", "confirmation.json", "child.json", "provider-counters.json", "answered-records.json", "replayed-confirmation.json", "replayed-records.json", "replayed-child.json", "replayed-parent.json"}
    verify_manifest(root, report.get("artifacts_sha256"), required)
    parent = json_proof(root, "parent.json")
    child = json_proof(root, "child.json")
    records = json_proof(root, "worker-records.json")["response"].get("records", [])
    question = next((r for r in records if r.get("kind") == "question"), {})
    proposal = next((r for r in records if r.get("kind") == "proposal"), {})
    answer = json_proof(root, "answer.json")
    confirmation = json_proof(root, "confirmation.json")
    counters = json_proof(root, "provider-counters.json")
    if len(records) != 2 or not question.get("id") or not proposal.get("id") or any(r.get("author") != "agent" for r in records) or parent.get("attempt", {}).get("safeReason") != "worker_question":
        raise ValueError("missing worker-origin question and model proposal")
    if json_proof(root, "unanswered-confirmation.json").get("status") != 409 or answer.get("request", {}).get("answer", {}).get("questionId") != question["id"] or confirmation.get("request", {}).get("proposalId") != proposal["id"]:
        raise ValueError("worker recovery bypassed answer and exact confirmation")
    attempt = child.get("attempt", {})
    if attempt.get("quality", {}).get("state") != "passed" or attempt.get("parentAttemptId") != parent.get("attempt", {}).get("attemptId") or attempt.get("recoveryProposalId") != proposal["id"] or confirmation.get("response", {}).get("confirmation", {}).get("attemptId") != attempt.get("attemptId"):
        raise ValueError("answered continuation lost its frozen recovery authority")
    if counters.get("calls") != 2 or counters.get("questions") != 1 or counters.get("continuations") != 1 or counters.get("answeredQuestionIds") != [question["id"]] or len(set(counters.get("requestIds", []))) != 2:
        raise ValueError("worker recovery repeated or omitted provider requests")
    for name, expected in (("child", child), ("parent", parent), ("confirmation", confirmation["response"]), ("records", json_proof(root, "answered-records.json")["response"])):
        if json_proof(root, f"replayed-{name}.json")["response"] != expected:
            raise ValueError(f"worker recovery changed retained {name} after restart")


def self_development(root, revision):
    value = record(root, "self-development/self-development.json", "build.ayni.orchestrator.self-development-acceptance.v1", {"own-source-managed-quality-and-draft", "own-source-queued-cancellation", "own-source-owner-restart-exact-replay"})
    quality = value.get("quality", {})
    if quality.get("state") != "passed" or quality.get("completedTargets") != quality.get("expectedTargets") or not quality.get("expectedTargets") or not quality.get("artifactDigest"):
        raise ValueError("incomplete own-source managed quality")
    if value.get("actual_source_repository") != "https://github.com/ayni-oss/orchestrator.git" or value.get("selected_source_paths") != ["README.md"] or value.get("source_commit") != revision:
        raise ValueError("self-development used another source or coding scope")
    for name in ("source_pack_digest", "lock_digest", "quality_digest", "patch_digest"):
        if not re.fullmatch(r"sha256:[a-f0-9]{64}", value.get(name, "")):
            raise ValueError(f"missing own-source identity: {name}")
    if not re.search(r"@sha256:[a-f0-9]{64}$", value.get("runtime_image", "")):
        raise ValueError("self-development runtime is not digest pinned")
    publication = value.get("publication", {})
    if not publication.get("protocol_fixture") or not publication.get("commit_sha") or not publication.get("pr_number") or value.get("live_provider") is not False or value.get("live_github") is not False:
        raise ValueError("self-development fixture publication boundary changed")


def build_report(root):
    root = Path(root).resolve()
    identity = json.loads(read(root, "source-identity.json"))
    revision = read(root, "source-revision.txt").strip()
    if not re.fullmatch(r"[a-f0-9]{40}", revision) or identity.get("revision") != revision or not identity.get("manifest") or not re.fullmatch(r"[a-f0-9]{64}", identity.get("lock_sha256", "")):
        raise ValueError("missing frozen repository/lock identity")
    m4 = record(root, "m4/acceptance.json", "m4-acceptance.v1")
    cases = m4.get("cases", {})
    if not M4_CASES.issubset(cases):
        raise ValueError("incomplete cumulative M4 failure cases")
    for case in cases.values():
        if case.get("result") != "passed" or not case.get("evidence"):
            raise ValueError("failed M4 case")
        for filename in case["evidence"]:
            if Path(filename).is_absolute() or ".." in Path(filename).parts:
                raise ValueError("M4 evidence escapes its profile")
            read(root / "m4", filename)
    m6 = record(root, "m6/acceptance.json", "build.ayni.orchestrator.task-recovery-acceptance.v1", M6_GATES)
    verify_manifest(root / "m6", m6.get("artifacts_sha256"))
    for profile in ("m4", "m6"):
        if read(root, f"{profile}/quality/source-revision.txt").strip() != revision or json.loads(read(root, f"{profile}/quality/source-manifest.json")) != identity["manifest"]:
            raise ValueError(f"{profile} source differs from cumulative frozen source")
    concurrency(root)
    for name in ("concurrency-adoption.txt", "operational-readiness.txt", "postgres-integration.txt", "private-v1-cutover.txt"):
        read(root, name)
    cutover = "private-v1-cutover: canonical-surface=pass stale-consumers=absent trusted-loopback=pass custom-listener=absent"
    if cutover not in read(root, "private-v1-cutover.txt"):
        raise ValueError("private-v1 obsolete-path cutover did not pass")
    if "postgres-image=" not in read(root, "postgres-integration.txt") or "test result: ok." not in read(root, "postgres-integration.txt"):
        raise ValueError("PostgreSQL retention integration did not complete")
    if "diagnostic passed" not in read(root, "conversation-diagnostic.txt"):
        raise ValueError("conversation diagnostic did not pass")
    if read(root, "readyz.txt").strip() != "ready":
        raise ValueError("deployed readiness did not pass")
    metrics = read(root, "metrics.txt")
    for name in ("workflow_database_ready", "workflow_task_sources_retired", "workflow_task_source_bytes_reclaimed", "workflow_task_sources_grace_period"):
        match = re.search(rf"^{name} ([0-9]+)$", metrics, re.MULTILINE)
        if not match or (name == "workflow_database_ready" and match.group(1) != "1"):
            raise ValueError(f"missing durable diagnostic metric: {name}")
    for name in ("workflow_task_runs", "workflow_task_source_bytes", "workflow_task_publications"):
        if not re.search(rf'^{name}\{{state="[a-z_]+"\}} [0-9]+$', metrics, re.MULTILINE):
            raise ValueError(f"missing lifecycle metric: {name}")
    hardware = json.loads(read(root, "hardware.json"))
    if not hardware or any(not node.get("capacity", {}).get("cpu") or not node.get("capacity", {}).get("memory") or not node.get("allocatable", {}).get("cpu") or not node.get("allocatable", {}).get("memory") or not node.get("system", {}).get("architecture") for node in hardware):
        raise ValueError("missing local capacity hardware metadata")
    inventories = sorted(root.glob("workloads-*.json"))
    if len(inventories) != 2:
        raise ValueError("missing deployed workload resource/image inventory")
    qualified = set()
    for inventory in inventories:
        for pod in json.loads(read(root, inventory.name)):
            images = {item.get("name"): item.get("image_id") for item in pod.get("image_ids", [])}
            for container in pod.get("containers", []):
                name = container.get("name")
                if name in ("control-plane", "task-supervisor"):
                    resources = container.get("resources", {})
                    if not pod.get("uid") or not container.get("image") or not images.get(name) or any(not resources.get(kind, {}).get(resource) for kind in ("requests", "limits") for resource in ("cpu", "memory")):
                        raise ValueError("missing control-plane/supervisor capacity or image identity")
                    qualified.add(name)
    if qualified != {"control-plane", "task-supervisor"}:
        raise ValueError("capacity evidence lacks deployed control-plane/supervisor")
    dependency = record(root, "dependencies/outcome.json", "build.ayni.orchestrator.dependency-acceptance.v1", DEPENDENCY_GATES)
    dependency_proofs(root / "dependencies", dependency)
    assessment = record(root, "assessment/scenario/outcome.json", "build.ayni.orchestrator.assessment-acceptance.v1", ASSESSMENT_GATES)
    if assessment.get("liveProvider") is not False or len(assessment.get("attempts", [])) != 2 or any(a.get("attempt", {}).get("quality", {}).get("state") != "passed" for a in assessment["attempts"]):
        raise ValueError("missing deployed assessment managed-quality evidence")
    if not (root / "assessment/scenario/assessment-console.png").is_file():
        raise ValueError("missing assessment browser evidence")
    for provider in ("openai", "anthropic"):
        gates = MCP_GATES | ({"anthropic-provider-uncertainty-and-cancellation"} if provider == "anthropic" else set())
        value = record(root, f"mcp-{provider}/outcome.json", "build.ayni.orchestrator.mcp-acceptance.v1", gates)
        if value.get("provider") != provider or value.get("live_provider") is not False:
            raise ValueError("MCP provider fixture boundary changed")
        verify_manifest(root / f"mcp-{provider}", value.get("artifacts_sha256"), {f"{stage}{suffix}" for stage in ("initial", "replay") for suffix in (".json", "-pod.json", "-pvc.json")})
        mcp_replay(root / f"mcp-{provider}", provider)
    worker = record(root, "worker-recovery/scenario/outcome.json", "build.ayni.orchestrator.worker-recovery-acceptance.v1", WORKER_RECOVERY_GATES)
    worker_proofs(root / "worker-recovery/scenario", worker)
    self_development(root, revision)
    completed = ["cumulative-M4-failure-matrix", "cumulative-independent-task-M6-failure-matrix", "four-isolated-active-attempts-and-two-queued", "cyclic-plan-rejection", "unselected-dependencies-block-preparation", "concurrent-supervisor-adoption", "six-task-cancellation-and-exact-replay-after-owner-restarts", "deployed-readiness-and-conversation-diagnostic", "private-v1-clean-slate-cutover", "accelerated-persisted-clock-source-retention-integration", "deployed-multi-upstream-integration-and-confirmed-repair", "deployed-outcome-assessment-and-proposal-review", "deployed-MCP-external-effect-recovery", "deployed-worker-question-and-model-proposal-recovery", "deployed-Anthropic-provider-protocol-recovery", "orchestrator-self-development-through-managed-quality-and-draft-publication"]
    unmeasured = ["live-additional-provider-contract-evidence", "sustained-production-capacity-envelope"]
    artifacts = {}
    for path in sorted(root.rglob("*")):
        if path.is_symlink():
            raise ValueError("evidence bundle contains a symbolic link")
        if path.is_file() and path.relative_to(root) not in (Path("acceptance.json"), Path("outcome.json")):
            artifacts[str(path.relative_to(root))] = hashlib.sha256(path.read_bytes()).hexdigest()
    return {"schema": SCHEMA, "result": "passed", "profile": "cumulative-local-operational-acceptance", "cumulative_profile_complete": True, "local_acceptance_complete": True, "milestone_complete": False,
            "source": {"revision": revision, "lock_sha256": identity["lock_sha256"]}, "completed_gates": completed, "unmeasured_gates": unmeasured,
            "capacity": {"active_slots": 4, "queued_attempts": 2, "workload": "six deterministic cancellation fixtures", "hardware": "hardware.json", "resources_and_images": [p.name for p in inventories], "sustained_throughput_measured": False},
            "fixture_boundaries": {"coding": "explicit deterministic fixture", "quality": "actual managed Ayni runtime", "publication": "real Git transport and GitHub protocol fixture", "providers": "OpenAI and Anthropic deterministic protocol fixtures", "live_provider": False, "live_github": False}, "artifacts_sha256": artifacts}


def main():
    root = Path(sys.argv[1])
    destination = root / "acceptance.json"
    report = build_report(root)
    with destination.open("x") as output:
        output.write(json.dumps(report, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
