"""The reporting harness must fail closed when selectors or provider evidence disappear."""
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("workflow_eval", Path(__file__).with_name("run.py"))
RUN = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RUN)


class EvaluationReports(unittest.TestCase):
    def test_zero_ignored_renamed_and_failing_tests_never_pass(self):
        for code, text in [(0, "running 0 tests"), (0, "test sample ... ignored"),
                           (1, "test sample ... ok"), (0, "test other ... ok")]:
            self.assertEqual(RUN.rust_result(code, text.encode(), "sample"), "failed")
        self.assertEqual(RUN.rust_result(0, b"test sample ... ok\n", "sample"), "passed")

    def test_unavailable_is_explicit_and_required_provider_fails(self):
        report = {"deterministic": [{"status": "passed"}],
                  "provider": {"status": "unavailable", "cases": []}}
        self.assertFalse(RUN.failed(report))
        self.assertTrue(RUN.failed(report, True))
        report["provider"] = {"status": "completed", "cases": [{"status": "failed"}]}
        self.assertTrue(RUN.failed(report))
        report["provider"]["cases"] = [{"status": "unavailable"}]
        self.assertTrue(RUN.failed(report))

    def test_provider_report_requires_exact_case_inputs_model_and_profile(self):
        scenarios = json.loads((RUN.ROOT / "evals/scenarios.json").read_text())
        report = {"schema_version": 1, "mode": "model", "model": "test",
                  "status": "completed", "cases": [
                      {"scenario": s, "request": {"model": "test", "profile": {"revision": "sha"}},
                       "status": "passed"} for s in scenarios]}
        RUN.validate_provider(report, "test")
        for field, value in [("cases", []), ("model", "other"), ("mode", "scripted")]:
            changed = copy.deepcopy(report)
            changed[field] = value
            with self.assertRaises(ValueError):
                RUN.validate_provider(changed, "test")
        changed = copy.deepcopy(report)
        changed["cases"][0]["scenario"]["input"] = "wrong input"
        with self.assertRaises(ValueError):
            RUN.validate_provider(changed, "test")

    def test_changed_or_deleted_inputs_invalidate_a_comparison(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(RUN, "ROOT", Path(directory)):
            source = Path(directory) / "fixture"
            source.write_bytes(b"version1")
            inputs = {"fixture": {"sha256": RUN.digest(b"version1")}}
            self.assertTrue(RUN.unchanged(inputs))
            source.write_bytes(b"version2")
            self.assertFalse(RUN.unchanged(inputs))
            source.unlink()
            self.assertFalse(RUN.unchanged(inputs))

    def test_timeout_stops_fixture_server_descendants(self):
        from unittest.mock import Mock
        process = Mock(pid=123, returncode=None)
        process.communicate.side_effect = [RUN.subprocess.TimeoutExpired("fixture", 1), (b"private output", b"")]
        with patch.object(RUN.subprocess, "Popen", return_value=process), patch.object(RUN.os, "killpg") as kill:
            code, output, _ = RUN.execute(["fixture"], timeout=1)
        kill.assert_called_once_with(123, RUN.signal.SIGKILL)
        self.assertIsNone(code)
        self.assertEqual(output, b"")

    def test_reports_are_private_and_raw_process_output_is_not_retained(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "report.json"
            RUN.write_json(path, {"status": "passed"})
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            self.assertEqual(json.loads(path.read_text()), {"status": "passed"})
        with patch.object(RUN.subprocess, "Popen", side_effect=OSError("private error")):
            code, output, _ = RUN.execute(["missing"])
        self.assertIsNone(code)
        self.assertEqual(output, b"")


if __name__ == "__main__":
    unittest.main()
