#!/usr/bin/env bash
# Conversation context/replay and schema migration on Docker Desktop's local Helm topology.
set -euo pipefail
export HELM_NO_PLUGINS=1
[[ $# == 2 ]] || { echo 'usage: dev/ctl test context CONTROL_PLANE_IMAGE WEB_IMAGE' >&2; exit 2; }
root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
source "$root/dev/lib/common.sh"
namespace="ayni-context-$$"
scratch=$(mktemp -d /tmp/ayni-context.XXXXXX)
created=false
forward=''
port=${ORCHESTRATOR_CONTEXT_TEST_PORT:-$(python3 - <<'PYPORT'
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
    helm uninstall context-validation --kube-context docker-desktop -n "$namespace" --ignore-not-found >/dev/null || status=1
    kubectl --context docker-desktop delete namespace "$namespace" --wait=false >/dev/null || status=1
    kubectl --context docker-desktop wait --for=delete "namespace/$namespace" --timeout=60s >/dev/null || status=1
  fi
  rm -rf "$scratch"
  exit "$status"
}
trap cleanup EXIT
trap 'echo "Context fixture failed at line $LINENO" >&2' ERR
trap 'exit 130' INT
trap 'exit 143' TERM
kubectl --context docker-desktop get nodes >/dev/null
dev_build_chart_dependencies
AYNI_CONTEXT_FIXTURE_PATH="$scratch/current.sqlite" AYNI_LEGACY_CONTEXT_FIXTURE_PATH="$scratch/legacy.sqlite" \
  cargo test --locked -p ayni-orchestrator-control-plane --lib export_project_context_fixture -- --ignored
AYNI_CONTEXT_REQUEST_PATH="$scratch/current.sqlite.request.json" \
  cargo test --locked -p ayni-orchestrator-worker --lib control_plane_context_fixture_matches_worker_contract -- --ignored
if kubectl --context docker-desktop get namespace "$namespace" >/dev/null 2>&1; then echo 'fixture namespace already exists' >&2; exit 1; fi
kubectl --context docker-desktop create namespace "$namespace"
created=true
kubectl --context docker-desktop label namespace "$namespace" orchestrator.ayni.dev/environment=local
helm upgrade --install context-validation "$DEV_CHART" \
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
reject() {
  if api "$1" "$2" >"$scratch/rejected.json" 2>"$scratch/rejected-error"; then echo "unexpected successful decision: $1" >&2; exit 1; fi
  rg -q "Code: $3" "$scratch/rejected-error" || { cat "$scratch/rejected-error" >&2; exit 1; }
}
stop
# Use the existing production web image's Node runtime to copy fixture bytes; no
# SQLite patching or state copying occurs while the control-plane writer runs.
cat >"$scratch/tooling.yaml" <<YAML
apiVersion: v1
kind: Pod
metadata: {name: context-fixture}
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
k wait --for=condition=Ready pod/context-fixture --timeout=120s
seed() {
  k exec context-fixture -- node -e 'const f=require("fs"); for(const n of ["orchestrator.sqlite-wal","orchestrator.sqlite-shm"]) f.rmSync("/state/"+n,{force:true});'
  k exec -i context-fixture -- node -e 'const f=require("fs");const s=f.createWriteStream("/state/orchestrator.sqlite");process.stdin.pipe(s);' < "$1"
}
# Assertions below use exporter-defined conversation IDs and public ProtoJSON.
inspect() {
  # The sole application writer is stopped. Checkpoint with the production web
  # image's SQLite runtime and require a fully drained WAL before copying bytes.
  k exec context-fixture -- node -e 'const f=require("fs"),{DatabaseSync}=require("node:sqlite");const d=new DatabaseSync("/state/orchestrator.sqlite");const [r]=d.prepare("PRAGMA wal_checkpoint(TRUNCATE)").all();if(r.busy!==0||r.log!==0||r.checkpointed!==0)throw Error("fixture WAL checkpoint did not drain");d.close();const wal="/state/orchestrator.sqlite-wal";if(f.existsSync(wal)&&f.statSync(wal).size!==0)throw Error("fixture WAL is not empty");process.stdout.write(f.readFileSync("/state/orchestrator.sqlite"));' > "$scratch/observed.sqlite"
  python3 "$root/dev/tests/inspect-context.py" "$1" "$scratch/observed.sqlite"
}
manifest="$scratch/current.sqlite.json"
conversation=$(jq -r '.conversationId' "$manifest")
project=$(jq -r '.projectId' "$manifest")
request=$(jq -nc --arg p "$project" --arg c "$conversation" '{projectId:$p,conversationId:$c}')
get() { api GetProjectConversation "$(jq -nc --arg id "$conversation" '{conversationId:$id}')"; }
for mode in current legacy; do
  echo "Validating $mode durable conversation context"
  seed "$scratch/$mode.sqlite"
  start
  get >"$scratch/$mode-initial.json"
  jq -e '.conversation.messages|length==16' "$scratch/$mode-initial.json" >/dev/null
  jq -e '.conversation.runs|any(.id=="context-final-run" and .state=="interrupted")' "$scratch/$mode-initial.json" >/dev/null
  api GetProjectConversationContext "$request" >"$scratch/$mode-context.json"
  jq -e '(.archiveRecordCount|tonumber)>80 and (.summary.version|tonumber)>0 and .summary.sourceWatermark==.sourceWatermark and (.summary.sourceRecordIds|length)>0' "$scratch/$mode-context.json" >/dev/null
  api ListProjectConversationHistory "$(jq -c '.+{query:"amber",limit:10}' <<<"$request")" >"$scratch/$mode-search.json"
  jq -e --arg id "$(jq -r '.firstRecordId' "$manifest")" '.records|any(.id==$id and (.content|contains("preserve the amber migration boundary")))' "$scratch/$mode-search.json" >/dev/null
  api ListProjectConversationHistory "$(jq -c '.+{limit:7}' <<<"$request")" >"$scratch/$mode-page.json"
  jq -e '.hasMore and (.records|length)==7' "$scratch/$mode-page.json" >/dev/null
  snapshot=$(jq -r '.snapshotSequence' "$scratch/$mode-page.json")
  before=$(jq -r '.nextBeforeSequence' "$scratch/$mode-page.json")
  api ListProjectConversationHistory "$(jq -nc --arg p "$project" --arg c "$conversation" --arg s "$snapshot" --arg b "$before" '{projectId:$p,conversationId:$c,snapshotSequence:$s,beforeSequence:$b,limit:7}')" >"$scratch/$mode-next.json"
  jq -e --slurpfile first "$scratch/$mode-page.json" '.snapshotSequence==$first[0].snapshotSequence and ([.records[].id] as $ids|all($first[0].records[].id; . as $id|$ids|index($id)|not))' "$scratch/$mode-next.json" >/dev/null
  reject ListProjectConversationHistory "$(jq -c '.projectId="project-outside-scope"' <<<"$request")" FailedPrecondition
  reject GetProjectConversationContext "$(jq -c '.projectId="project-outside-scope"' <<<"$request")" FailedPrecondition
  # Replaying an old archived message returns current state without requeueing research.
  api SendProjectMessage "$(jq -c '.firstMessage' "$manifest")" >"$scratch/$mode-message-replay.json"
  cmp "$scratch/$mode-initial.json" "$scratch/$mode-message-replay.json"
  approval=$(jq -c --arg c "$conversation" '{conversationId:$c,planId:.planId,version:.approvedVersion,decision:"approve",idempotencyKey:.approvedIdempotencyKey}' "$manifest")
  api DecideProjectPlan "$approval" >"$scratch/$mode-approval-replay.json"
  cmp "$scratch/$mode-initial.json" "$scratch/$mode-approval-replay.json"
  task_request=$(jq -c '{developmentTaskId:.taskId}' "$manifest")
  grpcurl -plaintext -max-time 10 -d "$task_request" "127.0.0.1:$port" ayni.orchestrator.v1.DevelopmentTaskService/GetDevelopmentTask >"$scratch/$mode-task.json"
  stop
  inspect "$scratch/$mode.sqlite"
  start
  get >"$scratch/$mode-restarted.json"
  cmp "$scratch/$mode-initial.json" "$scratch/$mode-restarted.json"
  api GetProjectConversationContext "$request" >"$scratch/$mode-restarted-context.json"
  cmp "$scratch/$mode-context.json" "$scratch/$mode-restarted-context.json"
  api ListProjectConversationHistory "$(jq -c '.+{query:"amber",limit:10}' <<<"$request")" >"$scratch/$mode-restarted-search.json"
  cmp "$scratch/$mode-search.json" "$scratch/$mode-restarted-search.json"
  grpcurl -plaintext -max-time 10 -d "$task_request" "127.0.0.1:$port" ayni.orchestrator.v1.DevelopmentTaskService/GetDevelopmentTask >"$scratch/$mode-restarted-task.json"
  cmp "$scratch/$mode-task.json" "$scratch/$mode-restarted-task.json"
  api SendProjectMessage "$(jq -c '.firstMessage' "$manifest")" >"$scratch/$mode-restarted-replay.json"
  cmp "$scratch/$mode-initial.json" "$scratch/$mode-restarted-replay.json"
  stop
  inspect "$scratch/$mode.sqlite"
done
echo 'Durable context, older retrieval, scope, replay, schema migration and restart persistence passed (no provider or model execution).'
