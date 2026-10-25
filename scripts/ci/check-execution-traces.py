#!/usr/bin/env python3
"""Verify bounded OTLP evidence from the optional, local trace collector.

This checks diagnostics, never workflow completion or provider qualification.
Pair it with retained browser/runtime acceptance for interruption/replay authority.
"""
import argparse
import base64
from collections import Counter, defaultdict
import json
from pathlib import Path
import re
import subprocess
import time

MAX_BYTES = 32 * 1024 * 1024
CONVERSATION_STAGES = {
    "api.message.admit", "outbox.publish", "supervisor.command.receive",
    "supervisor.session.execute", "session.pod.prepare", "session.pod.ready",
    "broker.report.accept",
}
TASK_STAGES = {
    "task.api.start", "task.supervisor.reconcile", "task.source.prepare",
    "task.coding.execute", "task.coding.accept", "task.quality.freeze",
    "task.quality.collect", "task.quality.accept", "task.api.publish",
    "task.publication.reconcile", "task.publication.accept",
}
REPLAY_STAGES = {"outbox.publish", "supervisor.command.receive", "broker.report.accept",
                 "worker.scripted.execute", "task.coding.execute", "task.publication.reconcile",
                 "provider.model.execute", "provider.tool.execute"}
FORBIDDEN = {"prompt", "api_key", "authorization", "content", "tool_arguments", "tool_output", "raw_error", "password", "secret", "body", "exception.message", "exception.stacktrace"}
WORKFLOW_FIELDS = {
    "stage", "workspace_id", "turn_id", "event_id", "provider_call_id", "tool_invocation_id",
    "parent_attempt_id", "replay", "project_id", "conversation_id", "message_id", "run_id",
    "plan_id", "plan_version", "proposal_id", "task_id", "attempt_id", "session_id", "generation",
    "model", "profile_revision", "outcome", "tool", "model_calls", "input_tokens", "output_tokens",
    "reason_code", "execution", "quality", "publication",
}
SERVICES = {"control-plane", "conversation-supervisor", "conversation-worker", "task-supervisor", "task-worker", "task-publisher", "workflow-migrate"}


def reject_content_attributes(value):
    if isinstance(value, dict):
        key = value.get("key", "")
        if isinstance(key, str) and (key.lower() in FORBIDDEN or FORBIDDEN.intersection(key.lower().split("."))):
            raise ValueError("content-bearing attribute found in exported workflow diagnostics")
        for nested in value.values():
            reject_content_attributes(nested)
    elif isinstance(value, list):
        for nested in value:
            reject_content_attributes(nested)


def projection(fields):
    selected = {}
    for key, value in fields.items():
        if key not in WORKFLOW_FIELDS:
            continue
        if isinstance(value, str) and (value == "[invalid]" or re.fullmatch(r"[a-zA-Z0-9_.:/@-]{0,128}", value)):
            selected[key] = value
        elif isinstance(value, (bool, int)):
            selected[key] = value
    return selected


def identity(value, size):
    if re.fullmatch(r"[0-9a-fA-F]{%d}" % (size * 2), value or ""):
        return value.lower()
    try:
        decoded = base64.b64decode(value, validate=True)
        return decoded.hex() if len(decoded) == size else ""
    except (ValueError, TypeError):
        return ""


def attributes(values):
    return {item["key"]: next(iter(item.get("value", {}).values()), None) for item in values}


def spans_from(raw):
    if len(raw) > MAX_BYTES:
        raise ValueError("trace evidence exceeds the 32-MiB extraction bound")
    spans = []
    for line in raw.splitlines():
        if not line.strip():
            continue
        document = json.loads(line)
        reject_content_attributes(document)
        for resource in document.get("resourceSpans", []):
            service = attributes(resource.get("resource", {}).get("attributes", [])).get("service.name", "")
            service = service if service in SERVICES else "[other]"
            for scope in resource.get("scopeSpans", []):
                for span in scope.get("spans", []):
                    fields = projection(attributes(span.get("attributes", [])))
                    if fields.get("stage"):
                        spans.append({"trace_id": identity(span.get("traceId"), 16),
                                      "span_id": identity(span.get("spanId"), 8),
                                      "parent_span_id": identity(span.get("parentSpanId"), 8),
                                      "service": service, "fields": fields,
                                      "links": [{"trace_id": identity(link.get("traceId"), 16),
                                                 "span_id": identity(link.get("spanId"), 8)}
                                                for link in span.get("links", [])]})
    if any(not span["trace_id"] or not span["span_id"] for span in spans):
        raise ValueError("no valid correlated workflow spans were exported")
    return spans


def inspect(spans, profile):
    grouped = defaultdict(list)
    for span in spans:
        grouped[span["trace_id"]].append(span)
    conversation = []
    tasks = []
    gaps = []
    for trace_id, members in grouped.items():
        stages = {span["fields"]["stage"] for span in members}
        if CONVERSATION_STAGES <= stages and stages.intersection({"worker.scripted.execute", "provider.model.execute"}):
            conversation.append(trace_id)
        if TASK_STAGES <= stages:
            tasks.append(trace_id)
    if not conversation:
        gaps.append("no single conversation trace spans API, outbox, broker, supervisor, pod readiness and worker")
    if profile == "full" and not tasks:
        gaps.append("no single task attempt trace spans coding, managed-quality collection and draft publication")
    repeated = Counter((span["trace_id"], span["fields"]["stage"]) for span in spans)
    replay = [span for span in spans if span["fields"]["stage"] in REPLAY_STAGES and span["fields"].get("replay") is True and span["links"]
              and repeated[(span["trace_id"], span["fields"]["stage"])] > 1]
    if profile == "full" and not replay:
        gaps.append("no repeated retained operation has an explicit causal trace link")
    interruptions = [span for span in spans if span["fields"].get("execution") == "cancelling"]
    if profile == "full" and not interruptions:
        gaps.append("no task cancellation/interruption reconciliation was observed")
    return {"passed": not gaps, "profile": profile, "span_count": len(spans),
            "conversation_trace_ids": sorted(conversation), "task_attempt_trace_ids": sorted(tasks),
            "linked_repeated_operations": len(replay), "interrupted_operations": len(interruptions),
            "provider_model_spans": sum(span["fields"]["stage"] == "provider.model.execute" for span in spans),
            "provider_tool_spans": sum(span["fields"]["stage"] == "provider.tool.execute" for span in spans),
            "stages": dict(sorted(Counter(span["fields"]["stage"] for span in spans).items())),
            "gaps": gaps,
            "authority": "diagnostics only; retained runtime acceptance proves outcomes and interruption recovery"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--namespace", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--profile", choices=["conversation", "full"], default="full")
    options = parser.parse_args()
    if not re.fullmatch(r"[a-z0-9]([-a-z0-9]*[a-z0-9])?", options.namespace) or len(options.namespace) > 54:
        raise SystemExit("invalid local acceptance namespace")
    options.output.mkdir(parents=True, exist_ok=False)
    deadline = time.monotonic() + 15
    while True:
        raw = subprocess.check_output([
        "kubectl", "--context", "docker-desktop", "--namespace", options.namespace,
        "exec", "deployment/workflow-traces", "-c", "reader", "--", "sh", "-ec",
        'for path in /data/traces*.jsonl; do if [ -f "$path" ]; then cat "$path"; fi; done',
        ], timeout=30)
        spans = spans_from(raw)
        result = inspect(spans, options.profile)
        if result["passed"] or time.monotonic() >= deadline:
            break
        time.sleep(1)
    # Store only the allowlisted correlation projection, never arbitrary telemetry.
    (options.output / "spans.json").write_text(json.dumps(spans, indent=2) + "\n")
    result = inspect(spans, options.profile)
    (options.output / "outcome.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result))
    if not result["passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
