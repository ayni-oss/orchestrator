#!/usr/bin/env python3
"""Verify synthetic trace search and real scrape queries in the disposable stack."""
import json
import secrets
import sys
import time
import urllib.parse
import urllib.request


grafana_port, collector_port, namespace = sys.argv[1:]
base = f"http://127.0.0.1:{grafana_port}"
trace_id = secrets.token_hex(16)


def request(url, body=None):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(url, data=data, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=10) as response:
        return json.load(response)


def eventually(check):
    deadline = time.monotonic() + 90
    while True:
        try:
            return check()
        except (AssertionError, OSError, KeyError) as error:
            if time.monotonic() >= deadline:
                raise AssertionError(f"telemetry validation timed out: {error}") from error
            time.sleep(2)


start = time.time_ns() - 2_000_000_000
# Both existing service names, connected by an actual parent span reference.
resources = []
for index, service in enumerate(["control-plane", "runtime-worker"]):
    span = {
        "traceId": trace_id,
        "spanId": f"{index + 1:016x}",
        "name": "workflow",
        "kind": 1,
        "startTimeUnixNano": str(start + index * 100_000_000),
        "endTimeUnixNano": str(start + (2 - index) * 1_000_000_000),
        "attributes": [
            {"key": "stage", "value": {"stringValue": "telemetry.fixture"}},
            {"key": "outcome", "value": {"stringValue": "failed"}},
        ],
        "status": {"code": 2, "message": "synthetic_failure"},
    }
    if index:
        span["parentSpanId"] = "0000000000000001"
    resources.append({
        "resource": {"attributes": [{"key": "service.name", "value": {"stringValue": service}}]},
        "scopeSpans": [{"scope": {"name": "telemetry-fixture"}, "spans": [span]}],
    })
request(f"http://127.0.0.1:{collector_port}/v1/traces", {"resourceSpans": resources})


def query_jaeger(query):
    result = request(base + "/api/ds/query", {
        "queries": [{"refId": "A", "datasource": {"type": "jaeger", "uid": "jaeger"}, **query}],
        "from": str(int(time.time() * 1000) - 3_600_000),
        "to": str(int(time.time() * 1000) + 1000),
    })["results"]["A"]
    assert not result.get("error"), result.get("error")
    return result.get("frames", [])


def trace_values(frames):
    return [value for frame in frames for column in frame.get("data", {}).get("values", []) for value in column]


def search(service, tags="", min_duration=""):
    return query_jaeger({"queryType": "search", "service": service, "tags": tags, "minDuration": min_duration, "limit": 50})


def check_search():
    for service in ["control-plane", "runtime-worker"]:
        assert trace_id in trace_values(search(service, "stage=telemetry.fixture outcome=failed", "500ms")), service
    assert trace_id not in trace_values(search("runtime-worker", "stage=does.not.exist"))
    assert trace_id not in trace_values(search("runtime-worker", min_duration="10s"))
    frames = query_jaeger({"query": trace_id})
    assert frames and frames[0].get("data", {}).get("values"), "missing span timeline"


eventually(check_search)
print("Synthetic connected trace: service/attribute/duration filters and trace timeline passed")


def prom_query(expr):
    url = base + "/api/datasources/proxy/uid/prometheus/api/v1/query?" + urllib.parse.urlencode({"query": expr})
    result = request(url)
    assert result["status"] == "success", result
    return result["data"]["result"]


def check_counters():
    for job in ["otel-collector", "jaeger", "cadvisor"]:
        assert prom_query(f'up{{job="ayni-orchestrator-{job}"}} == 1'), job
    for metric in ["otelcol_receiver_accepted_spans", "otelcol_exporter_sent_spans"]:
        assert prom_query(f'{metric}{{job="ayni-orchestrator-otel-collector"}} >= 2'), metric


eventually(check_counters)
print("Collector receives and exports spans; Prometheus scrapes Collector and Jaeger")

# Exercise each provisioned PromQL expression against the real Prometheus parser.
# Idle application panels may be empty. Infrastructure series must actually exist.
for uid, title in [("ayni-orchestrator-tasks-resources", "System Overview"), ("ayni-orchestrator-traces", "Execution Traces")]:
    dashboard = request(base + f"/api/dashboards/uid/{uid}")["dashboard"]
    assert dashboard["title"] == title
    for panel in dashboard["panels"]:
        for target in panel.get("targets", []):
            if "expr" in target:
                expr = target["expr"].replace("$__rate_interval", "1m").replace("$__range", "1h")
                assert "__AYNI_NAMESPACE__" not in expr
                values = prom_query(expr)
                if uid == "ayni-orchestrator-traces" or panel["id"] in [1, 2, 3, 4, 5]:
                    def check_values(expr=expr, title=panel["title"]):
                        assert prom_query(expr), f"empty dashboard query: {title}"
                    eventually(check_values)
def require_metric(metric):
    assert prom_query(f'{metric}{{namespace="{namespace}",container!="",container!="POD"}}'), metric


for metric in ["container_memory_usage_bytes", "container_spec_memory_limit_bytes", "container_memory_rss", "container_cpu_cfs_throttled_periods_total", "container_oom_events_total"]:
    eventually(lambda metric=metric: require_metric(metric))
print("Provisioned dashboards and real infrastructure query coverage passed")
