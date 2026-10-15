#!/usr/bin/env python3
"""Retain repeatable workflow evaluations; never retain subprocess logs or credentials."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import time
from datetime import datetime, timezone

ROOT = Path(__file__).resolve().parent.parent


def digest(data):
    return hashlib.sha256(data).hexdigest()


def execute(command, timeout=900, env=None):
    started = time.monotonic()
    try:
        process = subprocess.Popen(command, cwd=ROOT, env=env, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, start_new_session=True)
        try:
            output, _ = process.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            # Include fixture-server descendants so the next run can bind its ports.
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.communicate()
            return None, b"", round((time.monotonic() - started) * 1000)
        return process.returncode, output, round((time.monotonic() - started) * 1000)
    except OSError:
        return None, b"", round((time.monotonic() - started) * 1000)


def rust_result(code, output, expected):
    # A renamed selector or ignored test must never look like a passing scenario.
    text = output.decode("utf-8", errors="replace")
    matched = re.search(r"^test " + re.escape(expected) + r" \.\.\. ok$", text, re.MULTILINE)
    return "passed" if code == 0 and matched else "failed"


def write_json(path, value):
    temporary = path.with_suffix(".tmp")
    with temporary.open("x", encoding="utf-8") as output:
        json.dump(value, output, indent=2)
        output.write("\n")
    temporary.chmod(0o600)
    temporary.replace(path)


def deterministic(output_dir):
    cases = json.loads((ROOT / "evals/deterministic.json").read_text())
    results = []
    for case in cases:
        env = os.environ.copy()
        env.pop("AYNI_EVAL_REPORT_PATH", None)
        if case["id"] == "model-contract-fixtures":
            env["AYNI_EVAL_REPORT_PATH"] = str(output_dir / "scripted.json")
        command = ["cargo", "test", "--locked", "--lib", "-p", case["package"], case["test"],
                   "--", "--exact", "--nocapture"]
        code, output, latency = execute(command, env=env)
        status = rust_result(code, output, case["test"])
        result = dict(case, status=status, latency_ms=latency, command=command,
                      reason=None if status == "passed" else "fixture_failed_or_not_executed",
                      model=None, usage=None, usage_status="no_provider_fixture")
        results.append(result)
        print(f'{case["id"]}: {status}', flush=True)
    return results


def browser_case(skip):
    case = {"id": "browser-conversation-workflow", "mode": "browser_fixture", "model": None,
            "usage": None, "usage_status": "no_provider_fixture"}
    if skip:
        return dict(case, status="skipped", reason="explicit_skip", latency_ms=0)
    command = ["pnpm", "--dir", "apps/web", "exec", "playwright", "test",
               "project-conversations.spec.ts", "--project=chromium", "--reporter=json"]
    code, output, latency = execute(command)
    try:
        stats = json.loads(output)["stats"]
        passed = code == 0 and stats["expected"] == 6 and not any(stats[k] for k in ["unexpected", "flaky", "skipped"])
    except (ValueError, KeyError, TypeError):
        stats, passed = {}, False
    return dict(case, status="passed" if passed else "failed", latency_ms=latency,
                command=command, counts=stats,
                reason=None if passed else "browser_failed_or_not_executed")


def provider(args):
    if not args.provider_pod:
        return {"status": "unavailable", "reason": "provider_worker_not_selected", "cases": []}
    command = ["kubectl", "--context", "docker-desktop", "--namespace", args.namespace,
               "exec", args.provider_pod, "-c", "research", "--",
               "/usr/local/bin/ayni-orchestrator-worker", "workflow-eval", args.model]
    code, output, latency = execute(command, timeout=660)
    if code != 0 or len(output) > 2 * 1024 * 1024:
        return {"status": "unavailable", "reason": "provider_worker_unavailable", "cases": [],
                "latency_ms": latency}
    try:
        report = json.loads(output)
        validate_provider(report, args.model)
        return report
    except (ValueError, KeyError, TypeError):
        return {"status": "unavailable", "reason": "invalid_provider_report", "cases": []}


def validate_provider(report, model):
    if report["schema_version"] != 1 or report["mode"] != "model" or report["model"] != model:
        raise ValueError("report identity")
    expected = json.loads((ROOT / "evals/scenarios.json").read_text())
    cases = report["cases"]
    if report["status"] == "unavailable" and cases == []:
        return
    if [c["scenario"] for c in cases] != expected:
        raise ValueError("scenario version mismatch")
    for case in cases:
        if case["request"]["model"] != model or case["status"] not in ("passed", "failed", "unavailable"):
            raise ValueError("case identity")
        if not case["request"]["profile"]["revision"]:
            raise ValueError("profile missing")


def unchanged(inputs):
    try:
        return all(digest((ROOT / name).read_bytes()) == item["sha256"] for name, item in inputs.items())
    except OSError:
        return False


def failed(report, require_provider=False):
    fixtures_pass = all(c["status"] in ("passed", "skipped") for c in report["deterministic"])
    model = report["provider"]
    cases = model["cases"]
    provider_pass = all(c["status"] == "passed" for c in cases)
    provider_available = model["status"] != "unavailable" and bool(cases) and all(c["status"] != "unavailable" for c in cases)
    return not report.get("source_unchanged", True) or not fixtures_pass or not provider_pass or (require_provider and not provider_available)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / ".local/evals")
    parser.add_argument("--provider-pod", help="Explicit existing resident planner Pod; may incur provider charges")
    parser.add_argument("--namespace")
    parser.add_argument("--model")
    parser.add_argument("--require-provider", action="store_true")
    parser.add_argument("--skip-browser", action="store_true", help="Report browser coverage as skipped")
    args = parser.parse_args()
    if args.provider_pod and not (args.namespace and args.model):
        parser.error("--provider-pod requires --namespace and --model")
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")
    output_dir = args.output.resolve() / stamp
    output_dir.mkdir(mode=0o700, parents=True, exist_ok=False)
    code, revision, _ = execute(["git", "rev-parse", "HEAD"])
    inputs = {}
    cases = json.loads((ROOT / "evals/deterministic.json").read_text())
    sources = {c["source"] for c in cases} | {"evals/scenarios.json", "evals/deterministic.json", "Cargo.lock", "rust-toolchain.toml", "backend/libs/runtime/contracts/src/agent_profile.rs"}
    sources |= {str(p.relative_to(ROOT)) for p in (ROOT / "apps/web/e2e").glob("*fixture*") if p.is_file()}
    sources.add("apps/web/e2e/project-conversations.spec.ts")
    # Retain the actual implementation when evaluating an uncommitted worktree too.
    sources |= {str(p.relative_to(ROOT)) for folder in ["backend/libs/observability/src",
                "backend/workers/runtime-worker/src"] for p in (ROOT / folder).rglob("*.rs")}
    sources |= {"backend/servers/control-plane/src/adapters/sqlite/" + name for name in
                ["project_conversations.rs", "project_plans.rs", "project_sessions.rs",
                 "project_worker_evidence.rs", "agent_profiles.rs", "workflow_trace.rs"]}
    sources |= {"evals/run.py", "evals/compare.py", "backend/workers/runtime-worker/Cargo.toml"}
    for source in sorted(sources):
        data = (ROOT / source).read_bytes()
        inputs[source] = {"sha256": digest(data), "content": data.decode()}
    fingerprint = digest("".join(f"{key}:{value['sha256']}\n" for key, value in inputs.items()).encode())
    report = {"schema_version": 1, "source_fingerprint": fingerprint, "started_at": stamp,
              "revision": revision.decode().strip() if code == 0 else None,
              "inputs": inputs, "deterministic": deterministic(output_dir), "provider": provider(args)}
    browser = browser_case(args.skip_browser)
    report["deterministic"].append(browser)
    print(f'browser: {browser["status"]}', flush=True)
    report["source_unchanged"] = unchanged(inputs)
    report["failed"] = failed(report, args.require_provider or bool(args.provider_pod))
    write_json(output_dir / "report.json", report)
    print(f'provider: {report["provider"]["status"]}')
    print(f'report: {output_dir / "report.json"}')
    return int(report["failed"])


if __name__ == "__main__":
    sys.exit(main())
