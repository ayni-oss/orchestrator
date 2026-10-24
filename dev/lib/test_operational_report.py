#!/usr/bin/env python3
"""Partial, stale or corrupted evidence cannot become an operational closure claim."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("operational_report", Path(__file__).with_name("operational-report.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class OperationalReportTests(unittest.TestCase):
    def write(self, root, name, value):
        destination = root / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(value if isinstance(value, str) else json.dumps(value))

    def profile(self, root, directory, schema, gates, **fields):
        self.write(root, f"{directory}/retained.json", {"proof": "retained exact identity"})
        proof = root / directory / "retained.json"
        self.write(root, f"{directory}/outcome.json", {"schema": schema, "result": "passed", "gates": list(gates), "artifacts_sha256": {"retained.json": hashlib.sha256(proof.read_bytes()).hexdigest()}, **fields})

    def refresh_manifest(self, root, directory):
        base = root / directory
        report = json.loads((base / "outcome.json").read_text())
        report["artifacts_sha256"] = {str(p.relative_to(base)): hashlib.sha256(p.read_bytes()).hexdigest() for p in base.rglob("*") if p.is_file() and p.name != "outcome.json"}
        self.write(root, f"{directory}/outcome.json", report)

    def dependency_fixture(self, root):
        directory = "dependencies"
        source = "f" * 40
        upstreams = [{"taskId": f"up{n}", "attemptId": f"attempt{n}", "quality": {"state": "passed"}, "publication": {"state": "draft", "commitSha": str(n) * 40}} for n in range(2)]
        selected = [{"taskId": u["taskId"], "attemptId": u["attemptId"], "commitSha": u["publication"]["commitSha"]} for u in upstreams]
        parent = {"attemptId": "parent", "sourceCommit": source, "patch": "saved patch", "quality": {"state": "failed"}}
        child = {"attemptId": "child", "parentAttemptId": "parent", "sourceCommit": source, "savedInputPatch": "saved patch", "maxSeconds": 300, "maxTokens": 4096, "quality": {"state": "passed"}}
        draft = {**child, "publication": {"state": "draft"}}
        confirmation = {"confirmation": {"attemptId": "child"}}
        inputs = {"sourceCommit": source, "upstreamSelections": selected, "maxSeconds": 300, "maxTokens": 10000}
        for name, value in (("dependent-quality-failed", parent), ("dependent-repaired", child), ("dependent-repaired-draft", draft), ("repair-confirmation", {"response": confirmation}), ("repair-proposal", {"request": {"proposal": {"additionalSeconds": 300, "additionalTokens": 4096}}}), ("dependent-request", {"inputs": inputs})):
            self.write(root, f"{directory}/{name}.json", value)
        for n, value in enumerate(upstreams):
            self.write(root, f"{directory}/upstream-{n}-draft.json", value)
        for owner in ("control-plane", "task-supervisor", "workflow-postgres"):
            for name, value in (("confirmation", confirmation), ("parent", {"attempt": parent}), ("child", {"attempt": draft}), ("preparation", {"preparation": {"attemptId": "child", "inputs": {**inputs, "maxSeconds": 300, "maxTokens": 4096}}})):
                self.write(root, f"{directory}/{name}-replay-{owner}.json", {"response": value})
        report = json.loads((root / directory / "outcome.json").read_text())
        report.update(integrated_source_commit=source, upstream_selections=selected)
        self.write(root, f"{directory}/outcome.json", report)
        self.refresh_manifest(root, directory)

    def worker_fixture(self, root):
        directory = "worker-recovery/scenario"
        parent = {"attempt": {"attemptId": "worker-parent", "safeReason": "worker_question"}}
        child = {"attempt": {"attemptId": "worker-child", "parentAttemptId": "worker-parent", "recoveryProposalId": "proposal", "quality": {"state": "passed"}}}
        records = {"records": [{"id": "question", "kind": "question", "author": "agent"}, {"id": "proposal", "kind": "proposal", "author": "agent"}]}
        confirmation = {"request": {"proposalId": "proposal"}, "response": {"confirmation": {"attemptId": "worker-child"}}}
        fixtures = {"parent": parent, "child": child, "worker-records": {"response": records}, "unanswered-confirmation": {"status": 409}, "answer": {"request": {"answer": {"questionId": "question"}}}, "confirmation": confirmation, "answered-records": {"response": records}, "provider-counters": {"calls": 2, "questions": 1, "continuations": 1, "answeredQuestionIds": ["question"], "requestIds": ["one", "two"]}, "replayed-parent": {"response": parent}, "replayed-child": {"response": child}, "replayed-confirmation": {"response": confirmation["response"]}, "replayed-records": {"response": records}}
        for name, value in fixtures.items():
            self.write(root, f"{directory}/{name}.json", value)
        self.refresh_manifest(root, directory)

    def mcp_driver(self, provider, replay):
        success = {"failure_reason": "", "response_content": "retained fixture output", "provider_call": {"state": "completed", "provider": provider, "totalTokens": "40", "toolInvocations": [{"name": name, "state": "completed"} for name in ("mcp_fixture_read", "mcp_fixture_write")]}}
        effects = []
        for mode, reason, tool, state, counter in (("fixture:mcp-lost", "provider_tool_uncertain", "mcp_fixture_lost", "failed", "mcpLostEffects"), ("fixture:mcp-cancel", "cancelled", "mcp_fixture_cancel", "cancelled", "mcpCancelledEffects")):
            result = {"command_id": mode, "command_digest": "sha256:retained", "failure_reason": reason, "response_content": "", "provider_call": {"provider": provider, "state": state, "toolInvocations": [{"name": tool, "state": "uncertain"}]}}
            effects.append({"command_id": mode, "command_digest": "sha256:retained", "external_effects": 1, "effect_counter": counter, "mode": mode, "result": result})
        failures = [{"response_content": "", "provider_call": {"provider": provider, "state": state}} for state in ("uncertain", "cancelled")] if provider == "anthropic" else []
        return {"outcome": "passed", "profile": "deployed-provider-mcp-worker", "provider": provider, "replay": replay, "success_result": success, "external_effect_failures": effects, "provider_failures": failures, "command_id": "retained", "command_digest": "retained", "final_stats": {"calls": 0, "mcpReads": 0, "mcpEffects": 1, "mcpLostEffects": 1, "mcpCancelledEffects": 1}}

    def fixture(self, root):
        revision = "a" * 40
        manifest = {"README.md": "b" * 64}
        self.write(root, "source-revision.txt", revision)
        self.write(root, "source-identity.json", {"revision": revision, "lock_sha256": "c" * 64, "manifest": manifest})
        for profile in ("m4", "m6"):
            self.write(root, f"{profile}/quality/source-revision.txt", revision)
            self.write(root, f"{profile}/quality/source-manifest.json", manifest)
        self.write(root, "m4/proof.txt", "retained")
        self.write(root, "m4/acceptance.json", {"schema": "m4-acceptance.v1", "result": "passed", "cases": {case: {"result": "passed", "evidence": ["proof.txt"]} for case in module.M4_CASES}})
        self.profile(root, "m6", "build.ayni.orchestrator.task-recovery-acceptance.v1", module.M6_GATES)
        (root / "m6/outcome.json").rename(root / "m6/acceptance.json")
        evidence = {field: [list(f"{field}{n}".encode()) for n in range(6)] for field in ("requests", "receipts", "active", "cancellation_requests", "cancellations", "terminal")}
        evidence.update(scope={"project": "project"}, blocked_dependency=[1])
        evidence["active"] = [list(f"pod{n}".encode()) for n in range(4)] + [[10], [11]]
        for stage in ("active", "adopted", "terminal"):
            self.write(root, f"concurrency-{stage}.json", evidence)
        self.write(root, "concurrency-pods.json", {"items": [{"metadata": {"uid": f"pod{n}"}} for n in range(4)]})
        for owner in ("control-plane", "task-supervisor", "workflow-postgres"):
            self.write(root, f"concurrency-replay-{owner}.txt", module.REPLAY)
        for name in ("concurrency-adoption.txt", "operational-readiness.txt", "postgres-integration.txt", "private-v1-cutover.txt"):
            self.write(root, name, "passed")
        self.write(root, "private-v1-cutover.txt", "private-v1-cutover: canonical-surface=pass stale-consumers=absent trusted-loopback=pass custom-listener=absent")
        self.write(root, "postgres-integration.txt", "test result: ok.\npostgres-image=postgres:17.6")
        self.write(root, "conversation-diagnostic.txt", "workflow diagnostic passed")
        self.write(root, "readyz.txt", "ready")
        self.write(root, "metrics.txt", '\n'.join(['workflow_database_ready 1', 'workflow_task_sources_retired 0', 'workflow_task_source_bytes_reclaimed 0', 'workflow_task_sources_grace_period 0'] + [f'{name}{{state="completed"}} 6' for name in ('workflow_task_runs', 'workflow_task_source_bytes', 'workflow_task_publications')]))
        self.write(root, "hardware.json", [{"capacity": {"cpu": "4", "memory": "8Gi"}, "allocatable": {"cpu": "4", "memory": "8Gi"}, "system": {"architecture": "arm64"}}])
        for scope in ("parent", "sessions"):
            self.write(root, f"workloads-{scope}.json", [{"uid": "retained-pod", "containers": [{"name": name, "image": "local/workload:immutable", "resources": {kind: {"cpu": "1", "memory": "1Gi"} for kind in ("requests", "limits")}} for name in ("control-plane", "task-supervisor")], "image_ids": [{"name": name, "image_id": "sha256:retained"} for name in ("control-plane", "task-supervisor")]}])
        self.profile(root, "dependencies", "build.ayni.orchestrator.dependency-acceptance.v1", module.DEPENDENCY_GATES)
        self.profile(root, "worker-recovery/scenario", "build.ayni.orchestrator.worker-recovery-acceptance.v1", module.WORKER_RECOVERY_GATES)
        self.dependency_fixture(root)
        self.worker_fixture(root)
        self.write(root, "assessment/scenario/outcome.json", {"schema": "build.ayni.orchestrator.assessment-acceptance.v1", "result": "passed", "gates": list(module.ASSESSMENT_GATES), "liveProvider": False, "attempts": [{"attempt": {"quality": {"state": "passed"}}}] * 2})
        self.write(root, "assessment/scenario/assessment-console.png", "screenshot")
        for provider in ("openai", "anthropic"):
            gates = module.MCP_GATES | ({"anthropic-provider-uncertainty-and-cancellation"} if provider == "anthropic" else set())
            self.profile(root, f"mcp-{provider}", "build.ayni.orchestrator.mcp-acceptance.v1", gates, provider=provider, live_provider=False)
            for stage in ("initial", "replay"):
                self.write(root, f"mcp-{provider}/{stage}.json", self.mcp_driver(provider, stage == "replay"))
                self.write(root, f"mcp-{provider}/{stage}-pod.json", {"metadata": {"uid": stage}, "status": {"containerStatuses": [{"name": "session", "imageID": "sha256:identity"}]}})
                self.write(root, f"mcp-{provider}/{stage}-pvc.json", {"metadata": {"uid": "retained"}})
            self.refresh_manifest(root, f"mcp-{provider}")
        self.write(root, "self-development/self-development.json", {"schema": "build.ayni.orchestrator.self-development-acceptance.v1", "result": "passed", "gates": ["own-source-managed-quality-and-draft", "own-source-queued-cancellation", "own-source-owner-restart-exact-replay"], "quality": {"state": "passed", "completedTargets": 1, "expectedTargets": 1, "artifactDigest": "sha256:" + "b" * 64}, "actual_source_repository": "https://github.com/ayni-oss/orchestrator.git", "source_commit": revision, "selected_source_paths": ["README.md"], "source_pack_digest": "sha256:" + "b" * 64, "lock_digest": "sha256:" + "c" * 64, "quality_digest": "sha256:" + "d" * 64, "patch_digest": "sha256:" + "e" * 64, "runtime_image": "local/runtime@sha256:" + "f" * 64, "publication": {"protocol_fixture": True, "commit_sha": "f" * 40, "pr_number": 1}, "live_provider": False, "live_github": False})

    def test_complete_local_profile_preserves_external_qualification_boundary(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            report = module.build_report(root)
            self.assertTrue(report["local_acceptance_complete"])
            self.assertFalse(report["milestone_complete"])
            self.assertIn("live-additional-provider-contract-evidence", report["unmeasured_gates"])
            self.assertIn("self-development/self-development.json", report["artifacts_sha256"])
            self.assertFalse(report["capacity"]["sustained_throughput_measured"])

    def test_missing_required_profile_or_gate_fails(self):
        for filename in ("dependencies/outcome.json", "assessment/scenario/outcome.json", "mcp-anthropic/outcome.json", "self-development/self-development.json", "worker-recovery/scenario/outcome.json"):
            with self.subTest(filename=filename), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self.fixture(root)
                value = json.loads((root / filename).read_text())
                value["gates"] = []
                self.write(root, filename, value)
                with self.assertRaises(ValueError):
                    module.build_report(root)

    def test_corrupted_retained_artifact_and_escaping_manifest_fail(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            self.write(root, "mcp-openai/retained.json", "changed")
            with self.assertRaises(ValueError):
                module.build_report(root)
            with self.assertRaises(ValueError):
                module.verify_manifest(root, {"../outside": "a" * 64})

    def test_duplicate_attempts_changed_adoption_or_missing_pod_identity_fail(self):
        for mutation in ("duplicate", "changed", "missing-pod"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self.fixture(root)
                if mutation == "missing-pod":
                    self.write(root, "concurrency-pods.json", {"items": []})
                else:
                    filename = "concurrency-terminal.json" if mutation == "duplicate" else "concurrency-adopted.json"
                    value = json.loads((root / filename).read_text())
                    if mutation == "duplicate":
                        value["requests"][1] = value["requests"][0]
                    else:
                        value["active"][0] = [100]
                    self.write(root, filename, value)
                with self.assertRaises(ValueError):
                    module.build_report(root)

    def test_rehashed_incomplete_protocol_and_recovery_evidence_fails(self):
        cases = (("mcp-openai", "initial.json", "tool-uncertainty"), ("mcp-anthropic", "initial.json", "provider-failures"), ("mcp-openai", "initial.json", "missing-usage"), ("dependencies", "child-replay-control-plane.json", "child-replay"), ("worker-recovery/scenario", "provider-counters.json", "duplicate-call"))
        for directory, filename, mutation in cases:
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                self.fixture(root)
                value = json.loads((root / directory / filename).read_text())
                if mutation == "tool-uncertainty":
                    value["external_effect_failures"][0]["result"]["provider_call"]["toolInvocations"][0]["state"] = "completed"
                elif mutation == "provider-failures":
                    value["provider_failures"] = []
                elif mutation == "missing-usage":
                    value["success_result"]["provider_call"].pop("totalTokens")
                elif mutation == "child-replay":
                    value["response"]["attempt"]["attemptId"] = "other-child"
                else:
                    value["calls"] = 3
                self.write(root, f"{directory}/{filename}", value)
                self.refresh_manifest(root, directory)
                with self.assertRaises(ValueError):
                    module.build_report(root)

    def test_required_proof_must_have_fingerprint(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            filename = "mcp-openai/outcome.json"
            value = json.loads((root / filename).read_text())
            del value["artifacts_sha256"]["replay.json"]
            self.write(root, filename, value)
            with self.assertRaises(ValueError):
                module.build_report(root)

    def test_stale_source_or_live_fixture_claim_fails(self):
        for filename, field, value in (("m4/quality/source-revision.txt", None, "f" * 40), ("self-development/self-development.json", "source_commit", "f" * 40), ("mcp-anthropic/outcome.json", "live_provider", True)):
            with self.subTest(filename=filename), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self.fixture(root)
                data = json.loads((root / filename).read_text()) if field else value
                if field:
                    data[field] = value
                self.write(root, filename, data)
                with self.assertRaises(ValueError):
                    module.build_report(root)


if __name__ == "__main__":
    unittest.main()
