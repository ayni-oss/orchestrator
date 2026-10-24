"""The aggregate report must not hide missing or contradictory deployed evidence."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("recovery_report", Path(__file__).with_name("recovery-report.py"))
report = importlib.util.module_from_spec(spec)
spec.loader.exec_module(report)


class RecoveryReportTests(unittest.TestCase):
    def fixture(self, root):
        def write(name, value):
            p = root / name
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(value)
        for name in ["quality/source-revision.txt", "quality/source-manifest.json", "quality/image-identities.json",
                     "confirmation-restart.txt", "supervisor-adoption.txt", "continued-pod-loss.txt", "cleanup.txt", "redeploy.txt"]:
            write(name, "retained evidence\n")
        write("quality/outcome.json", json.dumps({"result": "passed"}))
        write("quality/recovery-console/outcome.json", json.dumps({"result": "passed", "cases": ["two-tab-confirmation", "pending-origin-after-navigation", "attention-resolution"]}))
        write("late-worker-exit.txt", "1\n")
        write("late-worker-fence.txt", "late-output-rejected=pass\n")
        for stage in ("before", "stopped", "after"):
            write(f"retained-{stage}.txt", "same-volume-and-secret-identities\n")
        for scenario in report.SCENARIOS:
            write(f"{scenario}-terminal.json", json.dumps({k: [1] for k in ["parent_request", "parent", "proposal", "confirmation_request", "confirmation", "terminal", "feedback", "cancellation_request", "cancellation"]}))
            for owner in report.OWNERS:
                write(f"{scenario}-replay-{owner}.txt", report.REPLAY + "\n")

    def test_passed_current_runtime_cannot_claim_milestone_closure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            result = report.build_report(root)
            self.assertEqual(result["result"], "passed")
            self.assertFalse(result["milestone_complete"])
            self.assertEqual(len(result["unmet_prerequisites"]), 3)
            self.assertFalse(result["fixture_boundaries"]["live_provider_recovery"])
            self.assertIn("quality/recovery-console/outcome.json", result["artifacts_sha256"])

    def test_each_missing_replay_prevents_an_aggregate_pass(self):
        for scenario in report.SCENARIOS:
            for owner in report.OWNERS:
                with self.subTest(scenario=scenario, owner=owner), tempfile.TemporaryDirectory() as directory:
                    root = Path(directory)
                    self.fixture(root)
                    (root / f"{scenario}-replay-{owner}.txt").unlink()
                    with self.assertRaises(FileNotFoundError):
                        report.build_report(root)

    def test_changed_storage_successful_late_worker_or_failed_browser_cannot_pass(self):
        for name, value in [("retained-after.txt", "replaced volume"), ("late-worker-exit.txt", "0"),
                            ("quality/recovery-console/outcome.json", '{"result":"failed","cases":[]}')]:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self.fixture(root)
                (root / name).write_text(value)
                with self.assertRaises(ValueError):
                    report.build_report(root)


if __name__ == "__main__":
    unittest.main()
