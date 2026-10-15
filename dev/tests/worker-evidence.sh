#!/usr/bin/env bash
# Accepted worker evidence, origin backfill, and restart recovery on Docker Desktop's local Helm topology.
set -euo pipefail
export HELM_NO_PLUGINS=1
[[ $# == 2 ]] || { echo 'usage: dev/ctl test worker-evidence CONTROL_PLANE_IMAGE WEB_IMAGE' >&2; exit 2; }
root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
source "$root/dev/lib/common.sh"
namespace="ayni-worker-evidence-$$"
scratch=$(mktemp -d /tmp/ayni-worker-evidence.XXXXXX)
created=false
forward=''
port=${ORCHESTRATOR_WORKER_EVIDENCE_TEST_PORT:-$(python3 - <<'PYPORT'
import socket
with socket.socket() as listener:
    listener.bind(('127.0.0.1', 0))
    print(listener.getsockname()[1])
PYPORT
)}
k() { kubectl --context docker-desktop -n "$namespace" "$@"; }
cleanup() {
  status=$?
  if [[ "$status" != 0 && "$created" == true ]]; then
    k get pods,persistentvolumeclaims -o wide >&2 || true
    k logs deployment/control-plane --tail=100 >&2 || true
    [[ ! -f "$scratch/forward.log" ]] || cat "$scratch/forward.log" >&2
  fi
  if [[ -n "$forward" ]]; then kill "$forward" 2>/dev/null || true; wait "$forward" 2>/dev/null || true; fi
  if [[ "$created" == true ]]; then
    helm uninstall worker-evidence-validation --kube-context docker-desktop -n "$namespace" --ignore-not-found >/dev/null || status=1
    kubectl --context docker-desktop delete namespace "$namespace" --wait=false >/dev/null || status=1
    kubectl --context docker-desktop wait --for=delete "namespace/$namespace" --timeout=60s >/dev/null || status=1
  fi
  rm -rf "$scratch"
  exit "$status"
}
trap cleanup EXIT
trap 'echo "Worker evidence fixture failed at line $LINENO" >&2' ERR
trap 'exit 130' INT
trap 'exit 143' TERM
kubectl --context docker-desktop get nodes >/dev/null
dev_build_chart_dependencies
AYNI_WORKER_EVIDENCE_FIXTURE_DIR="$scratch" cargo test --locked -p ayni-orchestrator-control-plane --lib export_worker_evidence_fixtures -- --ignored
if kubectl --context docker-desktop get namespace "$namespace" >/dev/null 2>&1; then echo 'fixture namespace already exists' >&2; exit 1; fi
kubectl --context docker-desktop create namespace "$namespace"
created=true
kubectl --context docker-desktop label namespace "$namespace" orchestrator.ayni.dev/environment=local
helm upgrade --install worker-evidence-validation "$DEV_CHART" \
  --kube-context docker-desktop -n "$namespace" --wait --timeout=5m \
  --set observability.enabled=false --set ayni-orchestrator.runtime.enabled=false \
  --set-string "ayni-orchestrator.images.controlPlane=$1" --set-string "ayni-orchestrator.images.web=$2" \
  --set ayni-orchestrator.images.pullPolicy=Never
stop() {
  if [[ -n "$forward" ]]; then kill "$forward" 2>/dev/null || true; wait "$forward" 2>/dev/null || true; forward=''; fi
  k scale deployment/control-plane --replicas=0
  k wait --for=delete pod -l app.kubernetes.io/name=ayni-orchestrator-control-plane --timeout=60s
}
start() {
  k scale deployment/control-plane --replicas=1
  k rollout status deployment/control-plane --timeout=180s
  kubectl --context docker-desktop -n "$namespace" port-forward --address=127.0.0.1 service/control-plane "$port:50051" >"$scratch/forward.log" 2>&1 &
  forward=$!
  for _ in {1..30}; do
    if ! kill -0 "$forward" 2>/dev/null; then cat "$scratch/forward.log" >&2; return 1; fi
    if rg -q "Forwarding from 127.0.0.1:$port" "$scratch/forward.log" && grpcurl -plaintext -max-time 2 "127.0.0.1:$port" list >/dev/null 2>&1; then return; fi
    sleep 1
  done
  cat "$scratch/forward.log" >&2
  return 1
}
api() { grpcurl -plaintext -max-time 10 -d "$2" "127.0.0.1:$port" "ayni.orchestrator.v1.ProjectConversationService/$1"; }

stop
# Use the existing production web image's Node runtime to copy fixture bytes; no
# SQLite patching or state copying occurs while the control-plane writer runs.
cat >"$scratch/tooling.yaml" <<YAML
apiVersion: v1
kind: Pod
metadata: {name: worker-evidence-fixture}
spec:
  restartPolicy: Never
  automountServiceAccountToken: false
  securityContext: {runAsNonRoot: true, runAsUser: 10001, runAsGroup: 10001, fsGroup: 10001, seccompProfile: {type: RuntimeDefault}}
  containers:
    - name: tooling
      image: $2
      imagePullPolicy: Never
      command: [node, -e, "setInterval(() => {}, 60000)"]
      resources: {requests: {cpu: 10m, memory: 32Mi}, limits: {cpu: 100m, memory: 128Mi}}
      securityContext: {allowPrivilegeEscalation: false, readOnlyRootFilesystem: true, capabilities: {drop: [ALL]}}
      volumeMounts: [{name: state, mountPath: /state}]
  volumes:
    - name: state
      persistentVolumeClaim: {claimName: control-plane-state}
YAML
k create -f "$scratch/tooling.yaml"
k wait --for=condition=Ready pod/worker-evidence-fixture --timeout=120s
seed() {
  k exec worker-evidence-fixture -- node -e 'const f=require("fs"); for(const n of ["orchestrator.sqlite-wal","orchestrator.sqlite-shm"]) f.rmSync("/state/"+n,{force:true});'
  k exec -i worker-evidence-fixture -- node -e 'const f=require("fs");const s=f.createWriteStream("/state/orchestrator.sqlite");process.stdin.pipe(s);' < "$1"
}
# Assertions below use exporter-defined conversation IDs and public ProtoJSON.
for mode in progress success failed-quality unavailable question cancelled publication-failed retry legacy; do
  echo "Validating $mode accepted worker evidence"
  manifest="$scratch/$mode.json"
  conversation=$(jq -r '.conversationId' "$manifest")
  project=$(jq -r '.projectId' "$manifest")
  task=$(jq -r '.taskId' "$manifest")
  get_request=$(jq -nc --arg id "$conversation" '{conversationId:$id}')
  context_request=$(jq -nc --arg p "$project" --arg c "$conversation" '{projectId:$p,conversationId:$c}')
  seed "$scratch/$mode.sqlite"
  start
  ready=false
  for _ in {1..30}; do
    api GetProjectConversation "$get_request" >"$scratch/$mode-observed.json"
    if jq -e --slurpfile m "$manifest" '(.conversation.workerUpdates|length)==$m[0].updates' "$scratch/$mode-observed.json" >/dev/null; then ready=true; break; fi
    sleep 1
  done
  [[ "$ready" == true ]] || { echo "worker evidence did not reconcile" >&2; exit 1; }
  jq -e --slurpfile m "$manifest" '.conversation as $c | ($c.runs|length)==1 and all($c.workerUpdates[]; .origin.projectId==$m[0].projectId and .origin.conversationId==$m[0].conversationId and .origin.planVersion=="1") and any($c.workerUpdates[]; .attemptId==$m[0].attemptId and (if $m[0].execution=="EXECUTION_OUTCOME_UNSPECIFIED" then .progress.phase=="WORKER_PHASE_CHECKING" else .result.execution==$m[0].execution and .result.quality.outcome==$m[0].quality and .result.publication==$m[0].publication end))' "$scratch/$mode-observed.json" >/dev/null
  grpcurl -plaintext -max-time 10 -d "$(jq -nc --arg id "$task" '{developmentTaskId:$id}')" "127.0.0.1:$port" ayni.orchestrator.v1.DevelopmentTaskService/GetDevelopmentTask >"$scratch/$mode-task.json"
  jq -e --arg c "$conversation" '.developmentTask.snapshot.origin.conversationId==$c and .developmentTask.snapshot.origin.planVersion=="1"' "$scratch/$mode-task.json" >/dev/null
  api GetProjectConversationContext "$context_request" >"$scratch/$mode-context.json"
  api ListProjectConversationHistory "$(jq -c '.+{query:"task_id",limit:100}' <<<"$context_request")" >"$scratch/$mode-history.json"
  jq -e 'any(.records[]; .kind=="worker_outcome")' "$scratch/$mode-history.json" >/dev/null
  stop
  start
  api GetProjectConversation "$get_request" >"$scratch/$mode-restarted.json"
  cmp "$scratch/$mode-observed.json" "$scratch/$mode-restarted.json"
  api GetProjectConversationContext "$context_request" >"$scratch/$mode-restarted-context.json"
  cmp "$scratch/$mode-context.json" "$scratch/$mode-restarted-context.json"
  stop
 done
 echo 'Accepted worker evidence, origin backfill, retries, and restart idempotency passed on Docker Desktop Kubernetes.'
