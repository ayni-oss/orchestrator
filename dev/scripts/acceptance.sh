#!/usr/bin/env bash
set -euo pipefail

readonly dev_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$dev_directory/lib/common.sh"
context="$(dev_context)"
namespace="$(dev_namespace)"
case "$namespace" in
  ayni-orchestrator-acceptance-*) ;;
  *) echo 'acceptance requires an isolated ayni-orchestrator-acceptance-* namespace' >&2; exit 2 ;;
esac
if [[ $# -lt 1 || $# -gt 2 || "$1" != /* || ( $# -eq 2 && "$2" != --plans-only ) ]]; then
  echo 'usage: dev/ctl acceptance ABSOLUTE_EVIDENCE_DIRECTORY [--plans-only]' >&2
  exit 2
fi
readonly evidence="$1"
readonly profile="${2:-full}"
if [[ -e "$evidence" ]]; then
  echo 'refusing to overwrite existing acceptance evidence' >&2
  exit 2
fi
mkdir -p "$evidence"
# Evidence is append-only for this run. Retained state/PVCs are never removed.
status=failed
scaled_down=
metrics_forward_pid=
cleanup() {
  if [[ -n "$metrics_forward_pid" ]]; then kill "$metrics_forward_pid" 2>/dev/null || true; fi
  if [[ -n "$scaled_down" ]]; then
    kubectl --context "$context" --namespace "$namespace" scale "$scaled_down" --replicas=1 >/dev/null 2>&1 || true
  fi
  printf '{"result":"%s","namespace":"%s","profile":"%s","cleanup":"retained_namespace_and_all_authoritative_stores"}\n' \
    "$status" "$namespace" "$profile" > "$evidence/outcome.json"
}
trap cleanup EXIT

git -C "$DEV_REPOSITORY_ROOT" rev-parse HEAD > "$evidence/source-revision.txt"
git -C "$DEV_REPOSITORY_ROOT" diff --no-ext-diff --binary HEAD > "$evidence/source-diff.patch"
git -C "$DEV_REPOSITORY_ROOT" ls-files --cached --others --exclude-standard -z | python3 -c '
import hashlib,json,pathlib,sys
root=pathlib.Path(sys.argv[1])
manifest={name:hashlib.sha256(root.joinpath(name).read_bytes()).hexdigest()
for name in sys.stdin.buffer.read().decode().split("\0") if name and root.joinpath(name).is_file()}
print(json.dumps(manifest,sort_keys=True,indent=2))
' "$DEV_REPOSITORY_ROOT" > "$evidence/source-manifest.json"
"$dev_directory/scripts/check.sh" > "$evidence/readiness.txt" 2>&1
kubectl --context "$context" --namespace "$namespace" get pods -o json | python3 -c '
import json,sys
pods=json.load(sys.stdin)
print(json.dumps([{ "pod":p["metadata"]["name"],"uid":p["metadata"]["uid"],
"containers":[{"name":c["name"],"image":c["image"],"image_id":c.get("imageID","")}
for c in p.get("status",{}).get("containerStatuses",[])]} for p in pods["items"]],indent=2))
' > "$evidence/image-identities.json"

pvc_identity() {
  kubectl --context "$context" --namespace "$namespace" get pvc -o json | python3 -c '
import json,sys
print(json.dumps(sorted([(p["metadata"]["name"],p["metadata"]["uid"],p["spec"].get("volumeName",""))
for p in json.load(sys.stdin)["items"]]),indent=2))'
}
pvc_identity > "$evidence/pvc-before.json"

control_pod() {
  kubectl --context "$context" --namespace "$namespace" get pods \
    -l app.kubernetes.io/name=ayni-orchestrator-control-plane \
    --sort-by=.metadata.creationTimestamp -o jsonpath='{.items[-1:].metadata.name}'
}
if [[ "$profile" == full ]]; then
pod="$(control_pod)"
kubectl --context "$context" --namespace "$namespace" exec "$pod" --container control-plane -- \
  /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 \
  > "$evidence/client-latency.json" 2> "$evidence/client-latency.stderr"

# Capture owned timings before any process restart resets them. Neither health
# diagnostics nor Prometheus exposition contains credentials or message bodies.
executor_pod="$(kubectl --context "$context" --namespace "$namespace" get pod -l app.kubernetes.io/name=ayni-orchestrator-scripted-executor -o jsonpath='{.items[0].metadata.name}')"
kubectl --context "$context" --namespace "$namespace" exec "$executor_pod" -- \
  cat /var/lib/ayni-executor/ledger.health.json > "$evidence/executor-health.json"
jetstream_pod="$(kubectl --context "$context" --namespace "$namespace" get pod -l app.kubernetes.io/name=workflow-jetstream -o jsonpath='{.items[0].metadata.name}')"
kubectl --context "$context" --namespace "$namespace" exec "$jetstream_pod" -- \
  wget -q -O - 'http://127.0.0.1:8222/jsz?streams=true&consumers=true&config=true' > "$evidence/broker-capacity.json"
port="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')"
kubectl --context "$context" --namespace "$namespace" port-forward --address 127.0.0.1 "pod/$pod" "$port:8080" \
  > "$evidence/metrics-port-forward.txt" 2>&1 &
metrics_forward_pid=$!
python3 - "$port" "$evidence" <<'PY'
from pathlib import Path
import json, math, re, sys, time, urllib.request
port, evidence = sys.argv[1], Path(sys.argv[2])
for attempt in range(50):
    try:
        with urllib.request.urlopen(f'http://127.0.0.1:{port}/metrics', timeout=2) as response:
            text=response.read().decode()
        break
    except OSError:
        if attempt == 49: raise
        time.sleep(.1)
evidence.joinpath('application-metrics.prom').write_text(text)
metrics={line.rsplit(' ',1)[0]:float(line.rsplit(' ',1)[1]) for line in text.splitlines() if line and not line.startswith('#')}
summary={}
for name, threshold in [
    ('workflow_receipt_handoff_seconds', .05),
    ('workflow_watch_notification_to_queue_seconds', .1),
    ('workflow_watch_live_commit_to_queue_seconds', .1),
    ('workflow_watch_live_terminal_commit_to_queue_seconds', .1),
]:
    count=metrics.get(name+'_count',0)
    within=metrics.get(name+f'_bucket{{le="{threshold}"}}',0)
    assert count>=100, (name, 'fewer than100samples', count)
    assert within>=math.ceil(count*.95), (name, 'p95targetfailed', within, count)
    summary[name]={'samples':count,'within_target':within,'target_seconds':threshold,'result':'passed'}
worker=json.loads(evidence.joinpath('executor-health.json').read_text())
for count_key, duration_key, limit in [
    ('command_claim_samples','command_claim_p95_seconds',.1),
    ('decision_terminal_samples','decision_terminal_p95_seconds',.25),
]:
    assert worker[count_key]>=100, (count_key,worker[count_key])
    assert worker[duration_key]<=limit, (duration_key,worker[duration_key],limit)
    summary[duration_key]={'samples':worker[count_key],'p95_seconds':worker[duration_key],'target_seconds':limit,'result':'passed'}
evidence.joinpath('owned-latency-gates.json').write_text(json.dumps(summary,indent=2)+'\n')
PY
kill "$metrics_forward_pid"
wait "$metrics_forward_pid" 2>/dev/null || true
metrics_forward_pid=

# A dependency outage is a correctness gate, never excluded silently as latency.
postgres_pod="$(kubectl --context "$context" --namespace "$namespace" get pod -l app.kubernetes.io/name=workflow-postgres -o jsonpath='{.items[0].metadata.name}')"
scaled_down=statefulset/workflow-postgres
kubectl --context "$context" --namespace "$namespace" scale "$scaled_down" --replicas=0 > "$evidence/postgres-outage.txt"
kubectl --context "$context" --namespace "$namespace" wait --for=delete "pod/$postgres_pod" --timeout=2m >> "$evidence/postgres-outage.txt"
pod="$(control_pod)"
kubectl --context "$context" --namespace "$namespace" exec -i "$pod" --container control-plane --   /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 --expect-unavailable-stdin   < "$evidence/client-latency.json" > "$evidence/postgres-unavailable.json"
kubectl --context "$context" --namespace "$namespace" scale "$scaled_down" --replicas=1 >> "$evidence/postgres-outage.txt"
kubectl --context "$context" --namespace "$namespace" rollout status "$scaled_down" --timeout=3m >> "$evidence/postgres-outage.txt"
scaled_down=

fi

# Accept FIFO work without a broker, restart the application before publication,
# then restore delivery. Authority and exact receipt bytes must survive both.
jetstream_pod="$(kubectl --context "$context" --namespace "$namespace" get pod -l app.kubernetes.io/name=workflow-jetstream -o jsonpath='{.items[0].metadata.name}')"
scaled_down=statefulset/workflow-jetstream
kubectl --context "$context" --namespace "$namespace" scale "$scaled_down" --replicas=0 > "$evidence/broker-outage.txt"
kubectl --context "$context" --namespace "$namespace" wait --for=delete "pod/$jetstream_pod" --timeout=2m >> "$evidence/broker-outage.txt"
pod="$(control_pod)"
# Review and approve while delivery is unavailable: PostgreSQL alone authorizes M3.
kubectl --context "$context" --namespace "$namespace" exec "$pod" --container control-plane -- \
  /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 --prepare-plans \
  > "$evidence/plans-before-restart.json"
if [[ "$profile" == full ]]; then
kubectl --context "$context" --namespace "$namespace" exec "$pod" --container control-plane --   /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 --prepare-pending   > "$evidence/pending-before-restart.json"
kubectl --context "$context" --namespace "$namespace" exec "$pod" --container control-plane -- \
  /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 --queue-capacity \
  > "$evidence/queue-capacity.json"
fi
kubectl --context "$context" --namespace "$namespace" rollout restart deployment/control-plane >> "$evidence/broker-outage.txt"
kubectl --context "$context" --namespace "$namespace" rollout status deployment/control-plane --timeout=3m >> "$evidence/broker-outage.txt"
pod="$(control_pod)"
kubectl --context "$context" --namespace "$namespace" exec -i "$pod" --container control-plane -- \
  /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 --verify-plans-stdin \
  < "$evidence/plans-before-restart.json" > "$evidence/plans-broker-unavailable.json"
kubectl --context "$context" --namespace "$namespace" scale "$scaled_down" --replicas=1 >> "$evidence/broker-outage.txt"
kubectl --context "$context" --namespace "$namespace" rollout status "$scaled_down" --timeout=3m >> "$evidence/broker-outage.txt"
scaled_down=
if [[ "$profile" == full ]]; then
pod="$(control_pod)"
kubectl --context "$context" --namespace "$namespace" exec -i "$pod" --container control-plane --   /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 --verify-pending-stdin   < "$evidence/pending-before-restart.json" > "$evidence/pending-after-restart.json"

fi

# Restart each process/dependency with retained volumes, then prove the original
# accepted receipt and response through canonical gRPC, without database repair.
for workload in deployment/control-plane statefulset/scripted-executor statefulset/workflow-jetstream statefulset/workflow-postgres; do
  name="${workload#*/}"
  kubectl --context "$context" --namespace "$namespace" rollout restart "$workload" \
    > "$evidence/restart-$name.txt"
  kubectl --context "$context" --namespace "$namespace" rollout status "$workload" --timeout=3m \
    >> "$evidence/restart-$name.txt"
  "$dev_directory/scripts/check.sh" >> "$evidence/restart-$name.txt" 2>&1
  pod="$(control_pod)"
  if [[ "$profile" == full ]]; then
  kubectl --context "$context" --namespace "$namespace" exec -i "$pod" --container control-plane -- \
    /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 --verify-stdin \
    < "$evidence/client-latency.json" > "$evidence/replay-$name.json"
  fi
  kubectl --context "$context" --namespace "$namespace" exec -i "$pod" --container control-plane -- \
    /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 --verify-plans-stdin \
    < "$evidence/plans-before-restart.json" > "$evidence/plans-replay-$name.json"
done
pvc_identity > "$evidence/pvc-after.json"
cmp "$evidence/pvc-before.json" "$evidence/pvc-after.json"
status=passed
printf 'acceptance profile=%s passed; evidence=%s\n' "$profile" "$evidence"
printf '%s\n' 'This command proves the named gates only. The complete M1 fault/capacity matrix also requires the repository integration gates.'
