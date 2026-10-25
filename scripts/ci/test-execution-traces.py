#!/usr/bin/env python3
"""Contract tests for the trace evidence checker, independent of Kubernetes."""
import importlib.util
import json
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("traces", Path(__file__).with_name("check-execution-traces.py"))
traces = importlib.util.module_from_spec(spec)
spec.loader.exec_module(traces)


class TraceEvidenceTests(unittest.TestCase):
    def span(self, stage, trace="1" * 32, **fields):
        return {"trace_id": trace, "span_id": "2" * 16, "fields": {"stage": stage, **fields}, "links": []}

    def test_unrelated_spans_cannot_satisfy_a_full_journey(self):
        spans = [self.span(stage, trace=f"{i + 1:032x}") for i, stage in enumerate(traces.CONVERSATION_STAGES)]
        spans.append(self.span("worker.scripted.execute"))
        self.assertFalse(traces.inspect(spans, "conversation")["passed"])

    def test_full_profile_requires_task_quality_publication_and_linked_replay(self):
        spans = [self.span(stage) for stage in traces.CONVERSATION_STAGES | {"worker.scripted.execute"}]
        self.assertTrue(traces.inspect(spans, "conversation")["passed"])
        self.assertFalse(traces.inspect(spans, "full")["passed"])
        spans += [self.span(stage, trace="3" * 32) for stage in traces.TASK_STAGES]
        poll = self.span("task.supervisor.reconcile", trace="3" * 32, replay=True, execution="cancelling")
        poll["links"] = [{"trace_id": "3" * 32, "span_id": "4" * 16}]
        spans.append(poll)
        self.assertFalse(traces.inspect(spans, "full")["passed"], "routine polls cannot establish replay")
        repeated = self.span("task.publication.reconcile", trace="3" * 32, replay=True)
        repeated["links"] = [{"trace_id": "3" * 32, "span_id": "4" * 16}]
        spans.append(repeated)
        self.assertTrue(traces.inspect(spans, "full")["passed"])

    def test_projection_drops_unknown_attributes_and_empty_export_can_retry(self):
        self.assertEqual(traces.projection({"stage": "api.message.admit", "unrecognized": "private", "task_id": "safe-id"}), {"stage": "api.message.admit", "task_id": "safe-id"})
        self.assertEqual(traces.spans_from(b""), [])
        self.assertFalse(traces.inspect([], "conversation")["passed"])
        with self.assertRaises(ValueError):
            traces.reject_content_attributes({"events": [{"attributes": [{"key": "authorization", "value": {"stringValue": "private"}}]}]})

    def test_parser_rejects_content_attributes_and_invalid_identities(self):
        raw = {"resourceSpans": [{"scopeSpans": [{"spans": [{"traceId": "1" * 32, "spanId": "2" * 16,
                "attributes": [{"key": "stage", "value": {"stringValue": "model"}},
                               {"key": "prompt", "value": {"stringValue": "private"}}]}]}]}]}
        with self.assertRaises(ValueError):
            traces.spans_from(json.dumps(raw).encode())
        self.assertEqual(traces.identity("malformed", 16), "")
        self.assertEqual(traces.identity("1" * 32, 16), "1" * 32)


if __name__ == "__main__":
    unittest.main()
