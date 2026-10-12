#!/usr/bin/env python3
"""Inspect only synthetic verification spans in the disposable validation namespace."""
import json
import sys
import time
import urllib.request
from pathlib import Path

port, context_file, grafana_port = sys.argv[1:]
trace_id = Path(context_file).read_text()[3:35]
trace = None
for attempt in range(30):
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{port}/api/traces/{trace_id}", timeout=2) as response:
            traces = json.load(response)["data"]
        if traces:
            candidate = traces[0]
            services = {p["serviceName"] for p in candidate["processes"].values()}
            stages = {t["value"] for s in candidate["spans"] for t in s["tags"] if t["key"] == "stage"}
            if {"runtime-worker", "control-plane", "verification-test-launcher"} <= services and {"worker", "delivery", "receipt", "decision", "cleanup"} <= stages:
                trace = candidate
                break
    except (OSError, ValueError, KeyError):
        pass
    time.sleep(1)
assert trace is not None, "connected worker, receipt, decision and restart-cleanup trace not found"
assert any(t["key"] == "reason_code" and t["value"] == "runtime_evidence_unavailable" for s in trace["spans"] for t in s["tags"])
assert any(s["operationName"] == "verification.report" for s in trace["spans"]), "worker shutdown did not flush report span"
assert any(r["refType"] == "FOLLOWS_FROM" for s in trace["spans"] for r in s["references"]), "recovery span links missing"
print(f"connected verification trace verified: {trace_id}; {len(trace['spans'])} spans")

# Exercise the shared UI backend, including service search (not only trace-by-ID).
base = f"http://127.0.0.1:{grafana_port}"
for attempt in range(30):
    try:
        with urllib.request.urlopen(f"{base}/api/datasources/uid/jaeger/health", timeout=2) as response:
            assert json.load(response)["status"] == "OK"
        break
    except OSError:
        if attempt == 29:
            raise
        time.sleep(1)
with urllib.request.urlopen(f"{base}/api/datasources/uid/jaeger/resources/services", timeout=2) as response:
    assert "runtime-worker" in json.load(response)
query = json.dumps({"queries": [{"refId": "A", "datasource": {"type": "jaeger", "uid": "jaeger"}, "query": trace_id}], "from": "0", "to": "now"}).encode()
request = urllib.request.Request(f"{base}/api/ds/query", data=query, headers={"Content-Type": "application/json"})
with urllib.request.urlopen(request, timeout=5) as response:
    result = json.load(response)["results"]["A"]
assert not result.get("error") and result["frames"], "Grafana could not display the connected trace"
with urllib.request.urlopen(f"{base}/api/datasources/uid/prometheus/health", timeout=2) as response:
    assert json.load(response)["status"] == "OK"
print("Grafana trace search/display and Prometheus datasource verified")
