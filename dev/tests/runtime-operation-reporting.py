#!/usr/bin/env python3
"""Exercise real supervisors and Pod-bound receipts in the disposable fixture."""
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import time

NAMESPACE = "ayni-orchestrator-reporting-test"
PREFIX = "runtime-operation-smoke-"


def kubectl(*args, data=None, capture=False):
    return subprocess.run(
        ["kubectl", "--context", "docker-desktop", "-n", NAMESPACE, *args],
        input=data, text=True, check=True, capture_output=capture,
    ).stdout


def inspect(path):
    connection = sqlite3.connect("file:" + path + "?mode=ro", uri=True)
    rows = connection.execute(
        "SELECT execution_id,decision,payload FROM runtime_operation_inbox WHERE execution_id LIKE 'runtime-operation-smoke-%' ORDER BY execution_id"
    ).fetchall()
    assert len(rows) == 4, rows
    # Protobuf payloads are retained; these synthetic profiles deliberately have
    # no selected generation and therefore must never advance runtime readiness.
    assert all(name.startswith(PREFIX) and state == "pending" and payload
               for name, state, payload in rows), rows
    accepted = connection.execute(
        "SELECT decision,stored_at,broker_sequence FROM runtime_operation_inbox WHERE execution_id='runtime-decision-smoke'"
    ).fetchall()
    if accepted:
        assert len(accepted) == 1 and accepted[0][0] == "accepted", accepted
        assert accepted[0][1] is not None and accepted[0][2] > 0, accepted
        print("accepted runtime fact acknowledged by real JetStream on the operation subject")
    print("four runtime receipts retained once across restart; no synthetic readiness")


def run(root, worker):
    binding = json.loads((Path(root) / "proto/testdata/verification/report.json").read_text())["data"]["binding"]
    binding.pop("resolvedImageDigest")
    pods = []
    for kind, operation in (("discovery", "registry-discover"), ("trust", "trust-lock"),
                            ("build", "build"), ("publication", "publish")):
        name = PREFIX + kind
        admission = {
            "configuration": {"source": "ayni://orchestrator/supervisors/validation",
                              "endpoint": "http://control-plane:8080/events/verification"},
            "binding": dict(binding, executionId=name, podUid=""), "kind": "RUNTIME_OPERATION_KIND_" + kind.upper(),
        }
        config = {"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": name},
                  "immutable": True, "data": {"operation-reporting.json": json.dumps(admission),
                                               "execution.json": "{}"}}
        kubectl("create", "-f", "-", data=json.dumps(config))
        expected = 0 if kind == "discovery" else 1
        # Invalid public inputs exercise terminal failure production without
        # provider credentials or network-dependent GitHub/GHCR side effects.
        script = f"""
/usr/local/bin/ayni-orchestrator-worker {operation}
test "$?" = {expected} || exit 1
test -s /output/runtime-operation-receipt.json || exit 1
cp /output/runtime-operation-event.json /output/original-event.json
cp /output/runtime-operation-receipt.json /output/original-receipt.json
while test ! -f /output/replay; do sleep 1; done
rm /output/runtime-operation-receipt.json
/usr/local/bin/ayni-orchestrator-worker report-operation || exit 1
cmp /output/original-event.json /output/runtime-operation-event.json || exit 1
cmp /output/original-receipt.json /output/runtime-operation-receipt.json || exit 1
echo runtime-operation-replay-passed
"""
        job = {"apiVersion": "batch/v1", "kind": "Job", "metadata": {"name": name},
               "spec": {"backoffLimit": 0, "activeDeadlineSeconds": 300,
                        "template": {"spec": {
                            "restartPolicy": "Never", "serviceAccountName": "runtime-coding",
                            "automountServiceAccountToken": False,
                            "securityContext": {"runAsUser": 10001, "runAsGroup": 10001, "fsGroup": 10001},
                            "initContainers": [{"name": "output", "image": worker, "imagePullPolicy": "Never",
                                                "args": ["prepare-output", name],
                                                "volumeMounts": [{"name": "output", "mountPath": "/outputs"}]}],
                            "containers": [{"name": "supervisor", "image": worker, "imagePullPolicy": "Never",
                                            "command": ["/bin/sh", "-c"], "args": [script],
                                            "securityContext": {"allowPrivilegeEscalation": False,
                                                                "readOnlyRootFilesystem": True,
                                                                "capabilities": {"drop": ["ALL"]}},
                                            "env": [{"name": "POD_UID", "valueFrom": {"fieldRef": {"fieldPath": "metadata.uid"}}}],
                                            "volumeMounts": [
                                                {"name": "request", "mountPath": "/request/operation-reporting.json",
                                                 "subPath": "operation-reporting.json", "readOnly": True},
                                                {"name": "request", "mountPath": "/request/execution.json",
                                                 "subPath": "execution.json", "readOnly": True},
                                                {"name": "output", "mountPath": "/output", "subPath": name},
                                                {"name": "identity", "mountPath": "/reporting", "readOnly": True}]}],
                            "volumes": [{"name": "request", "configMap": {"name": name}},
                                        {"name": "output", "persistentVolumeClaim": {"claimName": "runtime-outputs"}},
                                        {"name": "identity", "projected": {"defaultMode": 288, "sources": [
                                            {"serviceAccountToken": {"audience": "ayni-orchestrator-verification",
                                                                     "expirationSeconds": 600, "path": "token"}}]}}]}}}}
        kubectl("create", "-f", "-", data=json.dumps(job))
        pod = json.loads(kubectl("get", "pods", "-l", "job-name=" + name, "-o", "json", capture=True))["items"]
        for _ in range(60):
            if pod:
                break
            time.sleep(1)
            pod = json.loads(kubectl("get", "pods", "-l", "job-name=" + name, "-o", "json", capture=True))["items"]
        assert len(pod) == 1, pod
        pod_name = pod[0]["metadata"]["name"]
        kubectl("wait", "--for=condition=Ready", "pod/" + pod_name, "--timeout=60s")
        for attempt in range(60):
            try:
                kubectl("exec", pod_name, "-c", "supervisor", "--", "test", "-s",
                        "/output/runtime-operation-receipt.json", capture=True)
                break
            except subprocess.CalledProcessError:
                if attempt == 59:
                    kubectl("logs", pod_name, "-c", "supervisor")
                    raise
                time.sleep(1)
        pods.append((name, pod_name))
    print("all four operation receipts stored before supervisor exit", flush=True)
    kubectl("rollout", "restart", "deployment/control-plane")
    kubectl("rollout", "status", "deployment/control-plane", "--timeout=180s")
    for name, pod in pods:
        kubectl("exec", pod, "-c", "supervisor", "--", "touch", "/output/replay")
        kubectl("wait", "--for=condition=complete", "job/" + name, "--timeout=60s")
        kubectl("logs", "job/" + name, "-c", "supervisor")


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--inspect":
        inspect(sys.argv[2])
    elif len(sys.argv) == 3:
        run(sys.argv[1], sys.argv[2])
    else:
        raise SystemExit("usage: runtime-operation-reporting.py ROOT WORKER | --inspect SYNTHETIC_DB")
