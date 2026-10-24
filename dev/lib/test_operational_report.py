#!/usr/bin/env python3
"""A partial or corrupted evidence bundle cannot become a closure claim."""
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
        destination.write_text(value)

    def fixture(self, root):
        for profile in ("m4", "m6"):
            self.write(root, f"{profile}/acceptance.json", json.dumps({"result": "passed"}))
        evidence = {field: [[1]] * 6 for field in ("requests", "receipts", "active", "cancellation_requests", "cancellations", "terminal")}
        evidence["blocked_dependency"] = [1]
        self.write(root, "concurrency-terminal.json", json.dumps(evidence))
        for owner in ("control-plane", "task-supervisor", "workflow-postgres"):
            self.write(root, f"concurrency-replay-{owner}.txt", module.REPLAY)
        for name in ("concurrency-adoption.txt", "operational-readiness.txt", "postgres-integration.txt"):
            self.write(root, name, "passed")
        self.write(root, "conversation-diagnostic.txt", "workflow diagnostic passed")

    def test_success_preserves_unmeasured_gates_and_never_claims_closure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            report = module.build_report(root)
            self.assertEqual(report["result"], "passed")
            self.assertFalse(report["milestone_complete"])
            self.assertIn("orchestrator-self-development-through-managed-quality-and-draft-publication", report["unmeasured_gates"])
            self.assertIn("concurrency-terminal.json", report["artifacts_sha256"])
            self.assertIn("m4/acceptance.json", report["artifacts_sha256"])

    def test_self_development_requires_its_own_passing_managed_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            own_source = {"result": "passed", "quality": {"state": "passed"},
                          "actual_source_repository": "https://github.com/ayni-oss/orchestrator.git",
                          "selected_source_paths": ["README.md"], "publication": {"protocol_fixture": True},
                          "live_provider": False, "live_github": False}
            self.write(root, "self-development/self-development.json", json.dumps(own_source))
            report = module.build_report(root)
            gate = "orchestrator-self-development-through-managed-quality-and-draft-publication"
            self.assertIn(gate, report["completed_gates"])
            self.assertNotIn(gate, report["unmeasured_gates"])
            self.assertFalse(report["milestone_complete"])
            own_source["quality"]["state"] = "failed"
            self.write(root, "self-development/self-development.json", json.dumps(own_source))
            with self.assertRaises(ValueError):
                module.build_report(root)

    def test_missing_replay_or_partial_terminal_evidence_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            self.write(root, "concurrency-replay-control-plane.txt", "restarted")
            with self.assertRaises(ValueError):
                module.build_report(root)
            self.fixture(root)
            value = json.loads((root / "concurrency-terminal.json").read_text())
            value["terminal"].pop()
            self.write(root, "concurrency-terminal.json", json.dumps(value))
            with self.assertRaises(ValueError):
                module.build_report(root)


if __name__ == "__main__":
    unittest.main()
