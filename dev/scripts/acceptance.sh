#!/usr/bin/env bash
set -euo pipefail
if [[ $# -eq 1 || "${2:-}" == --quality-only ]]; then
  echo 'This acceptance profile includes retired browser coverage and is unavailable until UI acceptance is rebuilt.' >&2
  exit 2
fi


readonly dev_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$dev_directory/lib/common.sh"
if [[ $# -eq 2 && "$2" == --observable ]]; then
  exec "$dev_directory/scripts/observable-acceptance.sh" "$1"
fi
if [[ $# -eq 2 && "$2" == --m8 ]]; then
  exec "$dev_directory/scripts/operational-acceptance.sh" "$1"
fi
if [[ $# -eq 2 && "$2" == --publication-only ]]; then
  exec "$dev_directory/scripts/publication-acceptance.sh" "$1"
fi
if [[ $# -eq 2 && "$2" == --m6 ]]; then
  exec "$dev_directory/scripts/recovery-acceptance.sh" "$1"
fi
if [[ $# -eq 2 && "$2" == --m4 ]]; then
  exec "$dev_directory/scripts/publication-acceptance.sh" "$1" --m4
fi
context="$(dev_context)"
namespace="$(dev_namespace)"
case "$namespace" in
  ayni-orchestrator-acceptance-*) ;;
  *) echo 'acceptance requires an isolated ayni-orchestrator-acceptance-* namespace' >&2; exit 2 ;;
esac
if [[ $# -lt 1 || $# -gt 2 || "$1" != /* || ( $# -eq 2 && "$2" != --plans-only && "$2" != --tasks-only && "$2" != --coding-only && "$2" != --quality-only ) ]]; then
  echo 'usage: dev/ctl acceptance ABSOLUTE_EVIDENCE_DIRECTORY [--plans-only|--tasks-only|--coding-only|--quality-only]' >&2
  exit 2
fi
readonly evidence="$1"
readonly profile="${2:-full}"
quality_runtime="${AYNI_ORCHESTRATOR_QUALITY_RUNTIME_IMAGE:-}"
if [[ "$profile" == --quality-only ]]; then
  if [[ -z "$quality_runtime" && -f "$DEV_REPOSITORY_ROOT/.ayni/task-quality-runtime.json" ]]; then
    quality_runtime="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["runtime_image"])' "$DEV_REPOSITORY_ROOT/.ayni/task-quality-runtime.json")"
  fi
  [[ "$quality_runtime" =~ ^docker.io/local/ayni-task-quality@sha256:[a-f0-9]{64}$ ]] || {
    echo 'quality acceptance needs the local digest from dev/ctl quality-runtime' >&2; exit 2;
  }
fi
if [[ -e "$evidence" ]]; then
  echo 'refusing to overwrite existing acceptance evidence' >&2
  exit 2
fi
mkdir -p "$evidence"
# Evidence is append-only for this run. Retained state/PVCs are never removed.
status=failed
scaled_down=
fence_supervisor_stopped=false
fence_paused_container=
fence_node_id=
metrics_forward_pid=
cleanup() {
  if [[ -n "$fence_paused_container" ]]; then
    docker exec "$fence_node_id" ctr --namespace k8s.io tasks resume "$fence_paused_container" >/dev/null 2>&1 || true
  fi
  if [[ "$fence_supervisor_stopped" == true ]]; then
    kubectl --context "$context" --namespace "$namespace" scale deployment/task-supervisor --replicas=1 >/dev/null 2>&1 || true
  fi
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
pvc_expected="$evidence/pvc-before.json"

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
executor_pod="$(kubectl --context "$context" --namespace "$namespace" get pod -l app.kubernetes.io/name=ayni-orchestrator-conversation-supervisor -o jsonpath='{.items[0].metadata.name}')"
kubectl --context "$context" --namespace "$namespace" exec "$executor_pod" -- \
  cat /var/lib/ayni-executor/ledger.health.json > "$evidence/executor-health.json"
jetstream_pod="$(kubectl --context "$context" --namespace "$namespace" get pod -l app.kubernetes.io/name=jetstream -o jsonpath='{.items[0].metadata.name}')"
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
postgres_pod="$(kubectl --context "$context" --namespace "$namespace" get pod -l app.kubernetes.io/name=postgres -o jsonpath='{.items[0].metadata.name}')"
scaled_down=statefulset/postgres
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
jetstream_pod="$(kubectl --context "$context" --namespace "$namespace" get pod -l app.kubernetes.io/name=jetstream -o jsonpath='{.items[0].metadata.name}')"
scaled_down=statefulset/jetstream
kubectl --context "$context" --namespace "$namespace" scale "$scaled_down" --replicas=0 > "$evidence/broker-outage.txt"
kubectl --context "$context" --namespace "$namespace" wait --for=delete "pod/$jetstream_pod" --timeout=2m >> "$evidence/broker-outage.txt"
pod="$(control_pod)"
# Review and approve while delivery is unavailable: PostgreSQL alone authorizes M3.
kubectl --context "$context" --namespace "$namespace" exec "$pod" --container control-plane -- \
  /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 --prepare-plans \
  > "$evidence/plans-before-restart.json"
if [[ "$profile" != --plans-only ]]; then
kubectl --context "$context" --namespace "$namespace" exec "$pod" --container control-plane -- \
  /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 --prepare-execution \
  > "$evidence/execution-before-restart.json"
fi
if [[ "$profile" == --coding-only || "$profile" == --quality-only ]]; then
kubectl --context "$context" --namespace "$namespace" exec "$pod" --container control-plane -- \
  /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 --prepare-task-run \
  > "$evidence/task-run.json"
if [[ "$profile" == --quality-only ]]; then
  printf '%s\n' "$quality_runtime" > "$evidence/quality-runtime-image.txt"
  kubectl --context "$context" --namespace "$namespace" exec "$pod" --container control-plane -- \
    env "ORCHESTRATOR_TASK_QUALITY_RUNTIME_IMAGE=$quality_runtime" \
    /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 --prepare-task-quality-run \
    > "$evidence/task-quality-run.json"
  kubectl --context "$context" --namespace "$namespace" exec "$pod" --container control-plane -- \
    env "ORCHESTRATOR_TASK_QUALITY_RUNTIME_IMAGE=$quality_runtime" \
    /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 --prepare-task-quality-failed-run \
    > "$evidence/task-quality-failed-run.json"
fi
# Separate real worker cancellation and Pod-loss cases retain their own immutable origin.
for scenario in cancellation pod-loss; do
  kubectl --context "$context" --namespace "$namespace" exec "$pod" --container control-plane -- \
    /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 --prepare-task-run-pending \
    > "$evidence/task-$scenario.json"
  # The worker name is returned through the retained start receipt's protobuf identity.
  # Query this scenario's scoped attempt via the client helper rather than decoding protobuf in shell.
  kubectl --context "$context" --namespace "$namespace" exec -i "$pod" --container control-plane -- \
    /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 --task-name-stdin \
    < "$evidence/task-$scenario.json" > "$evidence/task-$scenario-name.txt"
  worker="$(cat "$evidence/task-$scenario-name.txt")"
  [[ "$worker" =~ ^task-[a-f0-9]{32}$ ]] || { echo 'invalid retained task worker name' >&2; exit 1; }
  for _ in $(seq 1 120); do
    if kubectl --context "$context" --namespace "${namespace}-sessions" exec "$worker" --container coding -- \
      test -f /workspace/task-result.txt >/dev/null 2>&1; then break; fi
    sleep .5
  done
  kubectl --context "$context" --namespace "${namespace}-sessions" exec "$worker" --container coding -- \
    cat /workspace/task-result.txt > "$evidence/task-$scenario-retained-source.txt"
  if [[ "$scenario" == cancellation ]]; then
    if [[ "${AYNI_ORCHESTRATOR_TASK_FENCE_ACCEPTANCE:-false}" == true ]]; then
      # Hold the real coding process before the supervisor restart. The fixed
      # fixture delay must not race slow rollout or node scheduling.
      [[ "$context" == docker-desktop ]] || { echo 'worker pause requires local Docker Desktop' >&2; exit 1; }
      kubectl --context "$context" --namespace "${namespace}-sessions" get pod "$worker" -o json > "$evidence/task-fence-worker-identity.json"
      fence_paused_container="$(python3 - "$evidence/task-fence-worker-identity.json" "$worker" "${namespace}-sessions" <<'PYIDENTITY'
import json,re,sys
pod=json.load(open(sys.argv[1]))
assert pod['metadata']['name']==sys.argv[2] and pod['metadata']['namespace']==sys.argv[3]
assert pod['spec']['nodeName']=='desktop-control-plane'
assert re.fullmatch(r'[a-f0-9-]{36}',pod['metadata']['uid'])
coding=[c for c in pod['status']['initContainerStatuses'] if c['name']=='coding']
assert len(coding)==1 and 'running' in coding[0]['state']
identity=coding[0]['containerID']
assert re.fullmatch(r'containerd://[a-f0-9]{64}',identity)
print(identity.removeprefix('containerd://'))
PYIDENTITY
)"
      fence_node_id="$(docker inspect desktop-control-plane --format '{{.Id}}')"
      [[ "$fence_node_id" =~ ^[a-f0-9]{64}$ ]] || exit 1
      [[ "$(docker inspect "$fence_node_id" --format '{{index .Config.Labels "io.x-k8s.kind.cluster"}}/{{index .Config.Labels "io.x-k8s.kind.role"}}')" == desktop/control-plane ]] || exit 1
      docker exec "$fence_node_id" ctr --namespace k8s.io tasks pause "$fence_paused_container"
      docker exec "$fence_node_id" ctr --namespace k8s.io tasks list | \
        awk -v id="$fence_paused_container" '$1==id {print $3}' > "$evidence/task-fence-worker-paused.txt"
      [[ "$(cat "$evidence/task-fence-worker-paused.txt")" == PAUSED ]] || exit 1
    fi
    before_uid="$(kubectl --context "$context" --namespace "${namespace}-sessions" get pod "$worker" -o jsonpath='{.metadata.uid}')"
    kubectl --context "$context" --namespace "$namespace" rollout restart deployment/task-supervisor > "$evidence/task-supervisor-active-restart.txt"
    kubectl --context "$context" --namespace "$namespace" rollout status deployment/task-supervisor --timeout=2m >> "$evidence/task-supervisor-active-restart.txt"
    after_uid="$(kubectl --context "$context" --namespace "${namespace}-sessions" get pod "$worker" -o jsonpath='{.metadata.uid}')"
    [[ "$before_uid" == "$after_uid" ]] || { echo 'supervisor restart replaced the active task' >&2; exit 1; }
    if [[ "${AYNI_ORCHESTRATOR_TASK_FENCE_ACCEPTANCE:-false}" == true ]]; then
      supervisor_pod="$(kubectl --context "$context" --namespace "$namespace" get pod -l app.kubernetes.io/name=ayni-task-supervisor -o jsonpath='{.items[0].metadata.name}')"
      [[ -n "$supervisor_pod" ]] || { echo 'supervisor Pod identity missing' >&2; exit 1; }
      kubectl --context "$context" --namespace "$namespace" scale deployment/task-supervisor --replicas=0 > "$evidence/task-fence-supervisor-stop.txt"
      fence_supervisor_stopped=true
      kubectl --context "$context" --namespace "$namespace" wait --for=delete pod -l app.kubernetes.io/name=ayni-task-supervisor --timeout=2m >> "$evidence/task-fence-supervisor-stop.txt"
      kubectl --context "$context" --namespace "$namespace" rollout status deployment/task-supervisor --timeout=2m >> "$evidence/task-fence-supervisor-stop.txt"
      kubectl --context "$context" --namespace "$namespace" exec -i "$pod" --container control-plane -- \
        /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 --cancel-task-run-pending-stdin \
        < "$evidence/task-$scenario.json" > "$evidence/task-fence-cancel.txt"
      # Resume the old worker only after the cancellation fence is retained.
      docker exec "$fence_node_id" ctr --namespace k8s.io tasks resume "$fence_paused_container"
      docker exec "$fence_node_id" ctr --namespace k8s.io tasks list | \
        awk -v id="$fence_paused_container" '$1==id {print $3}' > "$evidence/task-fence-worker-resumed.txt"
      [[ "$(cat "$evidence/task-fence-worker-resumed.txt")" == RUNNING ]] || exit 1
      fence_paused_container=
      # Its delayed result and bounded report retries must now be rejected.
      for _ in $(seq 1 360); do
        exit_code="$(kubectl --context "$context" --namespace "${namespace}-sessions" get pod "$worker" -o jsonpath='{.status.initContainerStatuses[?(@.name=="coding")].lastState.terminated.exitCode}')"
        [[ -z "$exit_code" ]] || break
        sleep .5
      done
      [[ "$exit_code" == 1 ]] || { echo 'stale coding worker did not reject its late result' >&2; exit 1; }
      printf '%s\n' "$exit_code" > "$evidence/task-fence-worker-exit.txt"
      kubectl --context "$context" --namespace "$namespace" exec -i "$pod" --container control-plane -- \
        /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 --verify-task-fenced-stdin \
        < "$evidence/task-$scenario.json" > "$evidence/task-fence-result.txt"
      kubectl --context "$context" --namespace "$namespace" scale deployment/task-supervisor --replicas=1 > "$evidence/task-fence-supervisor-start.txt"
      kubectl --context "$context" --namespace "$namespace" rollout status deployment/task-supervisor --timeout=2m >> "$evidence/task-fence-supervisor-start.txt"
      fence_supervisor_stopped=false
    fi
    mode=--cancel-task-run-stdin
  else
    kubectl --context "$context" --namespace "${namespace}-sessions" delete pod "$worker" --wait=true --timeout=60s > "$evidence/task-pod-loss-stop.txt"
    mode=--verify-interrupted-task-stdin
  fi
  kubectl --context "$context" --namespace "$namespace" exec -i "$pod" --container control-plane -- \
    /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 "$mode" \
    < "$evidence/task-$scenario.json" > "$evidence/task-$scenario-result.txt"
done
kubectl --context "$context" --namespace "${namespace}-sessions" get pvc -o json | python3 -c 'import json,sys; print(json.dumps(sorted((p["metadata"]["name"],p["metadata"]["uid"],p["spec"].get("volumeName","")) for p in json.load(sys.stdin)["items"])))' > "$evidence/task-pvc-before-delivery.json"
pvc_identity > "$evidence/pvc-after-task-provisioning.json"
pvc_expected="$evidence/pvc-after-task-provisioning.json"
fi
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
if [[ "$profile" != --plans-only ]]; then
kubectl --context "$context" --namespace "$namespace" exec -i "$pod" --container control-plane -- \
  /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 --verify-execution-stdin \
  < "$evidence/execution-before-restart.json" > "$evidence/execution-broker-unavailable.txt"
if [[ "$profile" == --coding-only || "$profile" == --quality-only ]]; then
kubectl --context "$context" --namespace "$namespace" exec -i "$pod" --container control-plane -- \
  /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 --verify-task-run-stdin \
  < "$evidence/task-run.json" > "$evidence/task-run-broker-unavailable.txt"
fi
fi
kubectl --context "$context" --namespace "$namespace" scale "$scaled_down" --replicas=1 >> "$evidence/broker-outage.txt"
kubectl --context "$context" --namespace "$namespace" rollout status "$scaled_down" --timeout=3m >> "$evidence/broker-outage.txt"
scaled_down=
if [[ "$profile" == --coding-only || "$profile" == --quality-only ]]; then
  # Broker recovery must settle every admitted continuity conversation before
  # freezing retained session storage. No PVC identity comparison is relaxed.
  pod="$(control_pod)"
  continuity_cases=(task-run)
  if [[ "$profile" == --quality-only ]]; then
    continuity_cases+=(task-quality-run task-quality-failed-run)
  fi
  for continuity_case in "${continuity_cases[@]}"; do
    kubectl --context "$context" --namespace "$namespace" exec -i "$pod" --container control-plane -- \
      /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 --settle-project-continuity-stdin \
      < "$evidence/$continuity_case.json" > "$evidence/continuity-$continuity_case-completed.txt"
  done
kubectl --context "$context" --namespace "${namespace}-sessions" get pvc -o json | python3 -c 'import json,sys; print(json.dumps(sorted((p["metadata"]["name"],p["metadata"]["uid"],p["spec"].get("volumeName","")) for p in json.load(sys.stdin)["items"])))' > "$evidence/task-pvc-before.json"
  python3 - "$evidence" <<'PYCODE'
from pathlib import Path
import json, sys
root=Path(sys.argv[1])
before=json.loads(root.joinpath('task-pvc-before-delivery.json').read_text())
after=json.loads(root.joinpath('task-pvc-before.json').read_text())
assert all(identity in after for identity in before), 'delivery recovery lost or replaced retained task/session PVCs'
root.joinpath('task-pvc-delivery-retention.json').write_text(json.dumps({
    'result':'passed','retained_identities':len(before),'settled_identities':len(after)
},indent=2)+'\n')
PYCODE
fi
if [[ "$profile" == full ]]; then
pod="$(control_pod)"
kubectl --context "$context" --namespace "$namespace" exec -i "$pod" --container control-plane --   /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 --verify-pending-stdin   < "$evidence/pending-before-restart.json" > "$evidence/pending-after-restart.json"

fi

# Restart each process/dependency with retained volumes, then prove the original
# accepted receipt and response through canonical gRPC, without database repair.
for workload in deployment/control-plane deployment/task-supervisor statefulset/conversation-supervisor statefulset/jetstream statefulset/postgres; do
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
  if [[ "$profile" != --plans-only ]]; then
  kubectl --context "$context" --namespace "$namespace" exec -i "$pod" --container control-plane -- \
    /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 --verify-execution-stdin \
    < "$evidence/execution-before-restart.json" > "$evidence/execution-replay-$name.txt"
  if [[ "$profile" == --coding-only || "$profile" == --quality-only ]]; then
  kubectl --context "$context" --namespace "$namespace" exec -i "$pod" --container control-plane -- \
    /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 --verify-interrupted-task-stdin \
    < "$evidence/task-pod-loss.json" > "$evidence/task-feedback-pod-loss-$name.txt"
  cmp "$evidence/task-pod-loss-result.txt" "$evidence/task-feedback-pod-loss-$name.txt"
  kubectl --context "$context" --namespace "$namespace" exec -i "$pod" --container control-plane -- \
    /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 --verify-task-run-stdin \
    < "$evidence/task-run.json" > "$evidence/task-run-replay-$name.txt"
  if [[ "$profile" == --quality-only ]]; then
  kubectl --context "$context" --namespace "$namespace" exec -i "$pod" --container control-plane -- \
    /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 --verify-task-quality-run-stdin \
    < "$evidence/task-quality-run.json" > "$evidence/task-quality-replay-$name.txt"
  kubectl --context "$context" --namespace "$namespace" exec -i "$pod" --container control-plane -- \
    /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 --verify-task-quality-failed-run-stdin \
    < "$evidence/task-quality-failed-run.json" > "$evidence/task-quality-failed-replay-$name.txt"
  fi
  fi
  fi
done
pvc_identity > "$evidence/pvc-after.json"
cmp "$pvc_expected" "$evidence/pvc-after.json"
if [[ "$profile" == --coding-only || "$profile" == --quality-only ]]; then
kubectl --context "$context" --namespace "${namespace}-sessions" get pvc -o json | python3 -c 'import json,sys; print(json.dumps(sorted((p["metadata"]["name"],p["metadata"]["uid"],p["spec"].get("volumeName","")) for p in json.load(sys.stdin)["items"])))' > "$evidence/task-pvc-after.json"
cmp "$evidence/task-pvc-before.json" "$evidence/task-pvc-after.json"
fi
if [[ "$profile" == --quality-only ]]; then
  "$dev_directory/scripts/task-console-acceptance.sh" "$evidence/task-quality-failed-run.json" "$evidence/recovery-console" --fresh-continuation
fi
status=passed
printf 'acceptance profile=%s passed; evidence=%s\n' "$profile" "$evidence"
printf '%s\n' 'This command proves the named gates only. The complete M1 fault/capacity matrix also requires the repository integration gates.'
