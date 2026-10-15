#!/usr/bin/env bash
# Real resident worker IPC plus explicitly seeded control-plane recovery boundaries.
set -euo pipefail
export HELM_NO_PLUGINS=1
[[ $# == 3 ]] || { echo 'usage: dev/ctl test sessions CONTROL_PLANE_IMAGE WEB_IMAGE WORKER_IMAGE' >&2; exit 2; }
root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
source "$root/dev/lib/common.sh"
source "$root/dev/lib/images.sh"
namespace="ayni-sessions-$$"
scratch=$(mktemp -d /tmp/ayni-sessions.XXXXXX)
created=false
forward=''
stub_forward=''
port=$(python3 - <<'PY'
import socket
with socket.socket() as listener:
    listener.bind(('127.0.0.1', 0))
    print(listener.getsockname()[1])
PY
)
stub_port=$(python3 - <<'PY'
import socket
with socket.socket() as listener:
    listener.bind(('127.0.0.1', 0))
    print(listener.getsockname()[1])
PY
)
k() { kubectl --context docker-desktop -n "$namespace" "$@"; }
cleanup() {
  status=$?
  if [[ "$status" != 0 && "$created" == true ]]; then
    k get pods -o wide >&2 || true
    k logs deployment/control-plane --tail=100 >&2 || true
    for pod in $(k get pods -l app.kubernetes.io/component=project-session -o name); do k logs "$pod" --all-containers --tail=50 >&2 || true; done
    [[ ! -f "$scratch/forward.log" ]] || cat "$scratch/forward.log" >&2
  fi
  for pid in "$forward" "$stub_forward"; do
    if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
  done
  if [[ "$created" == true ]]; then
    helm uninstall session-validation --kube-context docker-desktop -n "$namespace" --ignore-not-found >/dev/null || status=1
    kubectl --context docker-desktop delete namespace "$namespace" --wait=false >/dev/null || status=1
    kubectl --context docker-desktop wait --for=delete "namespace/$namespace" --timeout=60s >/dev/null || status=1
  fi
  rm -rf "$scratch"
  exit "$status"
}
trap cleanup EXIT
trap 'echo "Session fixture failed at line $LINENO" >&2' ERR
trap 'exit 130' INT
trap 'exit 143' TERM
kubectl --context docker-desktop get nodes >/dev/null
dev_build_chart_dependencies
# Fixture-only overlays route the unchanged production provider client to a local stub.
cat > "$scratch/Dockerfile" <<'DOCKER'
ARG BASE
FROM ${BASE}
ENV OPENAI_BASE_URL=http://planner-stub:8080/v1
DOCKER
cp_image="local/ayni-orchestrator-control-plane-session-fixture:$$"
worker_image="local/ayni-orchestrator-worker-session-fixture:$$"
docker build --build-arg "BASE=$1" -f "$scratch/Dockerfile" -t "$cp_image" "$scratch"
docker build --build-arg "BASE=$3" -f "$scratch/Dockerfile" -t "$worker_image" "$scratch"
for image in "$cp_image" "$worker_image" "$2"; do dev_load_image "$image"; done
worker_digest=$(dev_pin_worker_image "$worker_image")
if kubectl --context docker-desktop get namespace "$namespace" >/dev/null 2>&1; then echo 'fixture namespace already exists' >&2; exit 1; fi
kubectl --context docker-desktop create namespace "$namespace"
created=true
kubectl --context docker-desktop label namespace "$namespace" orchestrator.ayni.dev/environment=local
k create secret generic planner-model --from-literal=api-key=fixture-session-key-v1
k create secret generic planner-model-rotated --from-literal=api-key=fixture-session-key-v2
for secret in planner-model planner-model-rotated; do
  k label secret "$secret" ayni.dev/connection-id=session-fixture-connection ayni.dev/management-source=ui ayni.dev/purpose=connection
  k patch secret "$secret" --type=merge -p '{"immutable":true}'
done
k create configmap planner-stub --from-file=server.mjs="$root/dev/tests/planner-stub.mjs"
cat > "$scratch/stub.yaml" <<YAML
apiVersion: v1
kind: Pod
metadata: {name: planner-stub, labels: {app: planner-stub}}
spec:
  restartPolicy: Never
  automountServiceAccountToken: false
  securityContext: {runAsNonRoot: true, runAsUser: 10001, runAsGroup: 10001, fsGroup: 10001, seccompProfile: {type: RuntimeDefault}}
  containers:
    - name: server
      image: $2
      imagePullPolicy: Never
      command: [node, /fixture/server.mjs]
      ports: [{containerPort: 8080}]
      readinessProbe: {httpGet: {path: /healthz, port: 8080}, periodSeconds: 1}
      resources: {requests: {cpu: 10m, memory: 32Mi}, limits: {cpu: 200m, memory: 128Mi}}
      securityContext: {allowPrivilegeEscalation: false, readOnlyRootFilesystem: true, capabilities: {drop: [ALL]}}
      volumeMounts: [{name: script, mountPath: /fixture, readOnly: true}]
  volumes: [{name: script, configMap: {name: planner-stub}}]
---
apiVersion: v1
kind: Service
metadata: {name: planner-stub}
spec: {selector: {app: planner-stub}, ports: [{port: 8080, targetPort: 8080}]}
YAML
k create -f "$scratch/stub.yaml"
k wait --for=condition=Ready pod/planner-stub --timeout=120s
helm upgrade --install session-validation "$DEV_CHART" --kube-context docker-desktop -n "$namespace" --wait --timeout=5m \
  --set observability.enabled=false --set ayni-orchestrator.runtime.enabled=true \
  --set-string "ayni-orchestrator.images.controlPlane=$cp_image" --set-string "ayni-orchestrator.images.web=$2" \
  --set-string "ayni-orchestrator.runtime.workerImage=$worker_digest" --set ayni-orchestrator.runtime.intervalSeconds=2 \
  --set-string ayni-orchestrator.runtime.modelSecret=planner-model --set ayni-orchestrator.images.pullPolicy=Never
k auth can-i get pods/exec --as="system:serviceaccount:$namespace:control-plane" | rg -x yes
if k auth can-i get pods/exec --as="system:serviceaccount:$namespace:runtime-coding"; then echo 'runtime worker must not be able to exec into Pods' >&2; exit 1; fi
kubectl --context docker-desktop -n "$namespace" port-forward --address=127.0.0.1 service/planner-stub "$stub_port:8080" > "$scratch/stub-forward.log" 2>&1 &
stub_forward=$!
for _ in {1..30}; do if curl --fail --silent "http://127.0.0.1:$stub_port/healthz" >/dev/null; then break; fi; sleep 1; done
AYNI_SESSION_TEST_NAMESPACE="$namespace" AYNI_SESSION_TEST_IMAGE="$worker_digest" AYNI_SESSION_TEST_STUB="127.0.0.1:$stub_port" \
  cargo test --locked -p ayni-orchestrator-kubernetes --lib resident_planner_session_on_docker_desktop -- --ignored --nocapture
stop() {
  if [[ -n "$forward" ]]; then kill "$forward" 2>/dev/null || true; wait "$forward" 2>/dev/null || true; forward=''; fi
  k scale deployment/control-plane --replicas=0
  k wait --for=delete pod -l app.kubernetes.io/name=ayni-orchestrator-control-plane --timeout=60s
}
start() {
  k scale deployment/control-plane --replicas=1
  k rollout status deployment/control-plane --timeout=180s
  kubectl --context docker-desktop -n "$namespace" port-forward --address=127.0.0.1 service/control-plane "$port:50051" > "$scratch/forward.log" 2>&1 &
  forward=$!
  for _ in {1..30}; do
    if ! kill -0 "$forward" 2>/dev/null; then cat "$scratch/forward.log" >&2; return 1; fi
    if grpcurl -plaintext -max-time 2 "127.0.0.1:$port" list >/dev/null 2>&1; then return; fi
    sleep 1
  done
  cat "$scratch/forward.log" >&2; return 1
}
api() { grpcurl -plaintext -max-time 10 -d "$2" "127.0.0.1:$port" "ayni.orchestrator.v1.ProjectConversationService/$1"; }
reject() {
  if api "$1" "$2" > "$scratch/rejected.json" 2> "$scratch/rejected-error"; then echo "unexpected successful request: $1" >&2; return 1; fi
  rg -q "Code: $3" "$scratch/rejected-error" || { cat "$scratch/rejected-error" >&2; return 1; }
}
stop
cat > "$scratch/tooling.yaml" <<YAML
apiVersion: v1
kind: Pod
metadata: {name: session-fixture}
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
  volumes: [{name: state, persistentVolumeClaim: {claimName: control-plane-state}}]
YAML
k create -f "$scratch/tooling.yaml"
k wait --for=condition=Ready pod/session-fixture --timeout=120s
seed() {
  k exec session-fixture -- node -e 'const f=require("fs");for(const n of ["orchestrator.sqlite-wal","orchestrator.sqlite-shm"])f.rmSync("/state/"+n,{force:true});'
  k exec -i session-fixture -- node -e 'process.stdin.pipe(require("fs").createWriteStream("/state/orchestrator.sqlite"));' < "$scratch/state.sqlite"
}
snapshot() {
  k exec session-fixture -- node -e 'const f=require("fs"),{DatabaseSync}=require("node:sqlite");const d=new DatabaseSync("/state/orchestrator.sqlite");const[r]=d.prepare("PRAGMA wal_checkpoint(TRUNCATE)").all();if(r.busy!==0||r.log!==0||r.checkpointed!==0)throw Error("WAL did not drain");d.close();if(f.existsSync("/state/orchestrator.sqlite-wal")&&f.statSync("/state/orchestrator.sqlite-wal").size!==0)throw Error("WAL not empty");process.stdout.write(f.readFileSync("/state/orchestrator.sqlite"));' > "$scratch/state.sqlite"
}
mutate() {
  local secret=${2:-planner-model}
  local uid revision
  uid=$(k get secret "$secret" -o jsonpath='{.metadata.uid}')
  revision=$(k get secret "$secret" -o jsonpath='{.metadata.resourceVersion}')
  AYNI_SESSION_FIXTURE_PATH="$scratch/state.sqlite" AYNI_SESSION_FIXTURE_MODE="$1" AYNI_SESSION_MODEL_SECRET="$secret" AYNI_SESSION_MODEL_UID="$uid" AYNI_SESSION_MODEL_REVISION="$revision" \
    cargo test --locked -p ayni-orchestrator-control-plane --lib export_project_session_fixture -- --ignored
}
get() { api GetProjectConversation "$(jq -nc --arg id "$conversation" '{conversationId:$id}')"; }
wait_stub() {
  for _ in {1..60}; do
    if curl --fail --silent "http://127.0.0.1:$stub_port/fixture/stats" | jq -e '.pending==1' >/dev/null; then return; fi
    sleep 1
  done
  echo 'worker did not reach held model call' >&2; return 1
}
wait_state() {
  local expression=$1
  shift
  for _ in {1..90}; do get > "$scratch/conversation.json"; if jq -e "$expression" "$@" "$scratch/conversation.json" >/dev/null; then return; fi; sleep 1; done
  cat "$scratch/conversation.json" >&2; return 1
}
mutate seed
conversation=$(jq -r '.conversationId' "$scratch/state.json")
project=$(jq -r '.projectId' "$scratch/state.json")
seed; start
printf '%s\n' 'Control-plane phase uses valid pre-frozen empty-repository fixture requests; normal GitHub admission is covered separately.'
wait_state '.conversation.plannerSession.state=="idle" and (.conversation.runs|all(.state!="queued" and .state!="running"))'
workspace=$(jq -r '.workspaceId' "$scratch/state.json")
grpcurl -plaintext -max-time 10 -d "$(jq -nc --arg id "$workspace" '{workspaceId:$id}')" "127.0.0.1:$port" ayni.orchestrator.v1.ProjectService/GetWorkspace > "$scratch/workspace.json"
jq -e '.workspace.plannerIdleTimeoutSeconds==300' "$scratch/workspace.json" >/dev/null
workspace_settings=$(jq -c '{idempotencyKey:"fixture-workspace-timeout",workspaceId:.workspace.id,expectedResourceVersion:.workspace.resourceVersion,displayName:.workspace.displayName,plannerIdleTimeoutSeconds:86400}' "$scratch/workspace.json")
grpcurl -plaintext -max-time 10 -d "$workspace_settings" "127.0.0.1:$port" ayni.orchestrator.v1.ProjectService/UpdateWorkspace > "$scratch/workspace-updated.json"
get > "$scratch/conversation.json"
settings=$(jq -nc --arg p "$project" --arg c "$conversation" --arg cursor "$(jq -r '.conversation.updateCursor' "$scratch/conversation.json")" '{idempotencyKey:"fixture-override",projectId:$p,conversationId:$c,expectedUpdateCursor:$cursor,plannerIdleTimeoutSeconds:300}')
api UpdateProjectConversationSettings "$settings" > "$scratch/override.json"
jq -e '.conversation.plannerIdleTimeoutSeconds==300 and .conversation.effectivePlannerIdleTimeoutSeconds==300' "$scratch/override.json" >/dev/null
inherit=$(jq -nc --arg p "$project" --arg c "$conversation" --arg cursor "$(jq -r '.conversation.updateCursor' "$scratch/override.json")" '{idempotencyKey:"fixture-inherit",projectId:$p,conversationId:$c,expectedUpdateCursor:$cursor,inheritWorkspaceTimeout:true}')
api UpdateProjectConversationSettings "$inherit" > "$scratch/inherit.json"
jq -e '.conversation.plannerIdleTimeoutSeconds==null and .conversation.effectivePlannerIdleTimeoutSeconds==86400' "$scratch/inherit.json" >/dev/null
reject UpdateProjectConversationSettings "$(jq -c '.+{idempotencyKey:"fixture-stale"}' <<<"$settings")" Aborted
reject UpdateProjectConversationSettings "$(jq -c '.+{idempotencyKey:"fixture-foreign",projectId:"project:foreign"}' <<<"$inherit")" FailedPrecondition
first_generation=$(jq -r '.conversation.plannerSession.generation' "$scratch/conversation.json")
first_session=$(jq -r '.conversation.plannerSession.sessionId' "$scratch/conversation.json")
first_pods=$(k get pods -o json | jq -c '[.items[]|select(.metadata.name|startswith("project-"))|select(.metadata.name!="planner-stub")|.metadata.uid]')
jq -e 'length==1' <<<"$first_pods" >/dev/null
stop; snapshot; start
wait_state '.conversation.plannerSession.state=="idle" and (.conversation.runs|all(.state!="queued" and .state!="running"))'
[[ $(jq -r '.conversation.plannerSession.sessionId' "$scratch/conversation.json") == "$first_session" ]]
[[ $(k get pods -o json | jq -c '[.items[]|select(.metadata.name|startswith("project-"))|select(.metadata.name!="planner-stub")|.metadata.uid]') == "$first_pods" ]]
stop; snapshot; mutate expire; seed; start
wait_state '.conversation.plannerSession.state=="expired"'
stop; snapshot; mutate next; seed; start
wait_state '.conversation.plannerSession.state=="idle" and (.conversation.runs|all(.state!="queued" and .state!="running"))'
jq -e --arg generation "$first_generation" '(.conversation.plannerSession.generation|tonumber)>($generation|tonumber)' "$scratch/conversation.json" >/dev/null
rotation_generation=$(jq -r '.conversation.plannerSession.generation' "$scratch/conversation.json")
# Rotation selects a new binding without exposing a real provider credential.
stop; snapshot; mutate rotate planner-model-rotated; seed; start
wait_state '.conversation.plannerSession.state=="idle" and (.conversation.runs|all(.state!="queued" and .state!="running"))'
jq -e --arg generation "$rotation_generation" '(.conversation.plannerSession.generation|tonumber)>($generation|tonumber)' "$scratch/conversation.json" >/dev/null
# Losing a real active Pod interrupts the retained run instead of silently retrying it.
curl --fail --silent -X POST "http://127.0.0.1:$stub_port/fixture/hold" >/dev/null
stop; snapshot; mutate next planner-model-rotated; seed; start
wait_state '.conversation.plannerSession.state=="active"'
wait_stub
lost_run=$(jq -r '.conversation.plannerSession.activeRunId' "$scratch/conversation.json")
cp "$scratch/conversation.json" "$scratch/active-before.json"
active_pod=$(k get pods -l app.kubernetes.io/component=project-session -o jsonpath='{.items[0].metadata.uid}')
active_calls=$(curl --fail --silent "http://127.0.0.1:$stub_port/fixture/stats" | jq '.calls')
# Simulate a crash after resident ACK but before recording running; preserve all frozen fences.
stop; snapshot; mutate requeue planner-model-rotated; seed; start
wait_state '.conversation.plannerSession.state=="active" and (.conversation.runs|any(.id==$run and .state=="running"))' --arg run "$lost_run"
[[ $(k get pods -l app.kubernetes.io/component=project-session -o jsonpath='{.items[0].metadata.uid}') == "$active_pod" ]]
for _ in {1..3}; do
  get > "$scratch/active-after.json"
  jq -c --arg run "$lost_run" '.conversation.runs[]|select(.id==$run)|{id,sessionId,sessionGeneration,sessionTurnSequence,sessionDeadlineAt}' "$scratch/active-before.json" > "$scratch/fence-before.json"
  jq -c --arg run "$lost_run" '.conversation.runs[]|select(.id==$run)|{id,sessionId,sessionGeneration,sessionTurnSequence,sessionDeadlineAt}' "$scratch/active-after.json" > "$scratch/fence-after.json"
  cmp "$scratch/fence-before.json" "$scratch/fence-after.json"
  [[ $(curl --fail --silent "http://127.0.0.1:$stub_port/fixture/stats" | jq '.calls') == "$active_calls" ]]
  sleep 1
done
jq -e --arg run "$lost_run" '.conversation.runs|any(.id==$run and .state=="running")' "$scratch/active-after.json" >/dev/null
k delete pods -l app.kubernetes.io/component=project-session --wait=true --timeout=90s
wait_state '.conversation.plannerSession.state=="lost"'
jq -e --arg run "$lost_run" '.conversation.runs|any(.id==$run and .state=="interrupted")' "$scratch/conversation.json" >/dev/null
curl --fail --silent -X POST "http://127.0.0.1:$stub_port/fixture/release" >/dev/null
# Hold the actual worker at its provider boundary, then cancel via the real API.
curl --fail --silent -X POST "http://127.0.0.1:$stub_port/fixture/hold" >/dev/null
stop; snapshot; mutate next planner-model-rotated; seed; start
wait_state '.conversation.plannerSession.state=="active"'
wait_stub
run=$(jq -r '.conversation.plannerSession.activeRunId' "$scratch/conversation.json")
cancel=$(jq -nc --arg p "$project" --arg c "$conversation" --arg r "$run" '{idempotencyKey:"fixture-cancel",projectId:$p,conversationId:$c,runId:$r}')
reject CancelProjectAgentRun "$(jq -c '.+{idempotencyKey:"fixture-foreign-cancel",projectId:"project:foreign"}' <<<"$cancel")" FailedPrecondition
assistant_count=$(jq '.conversation.messages|map(select(.role=="assistant"))|length' "$scratch/conversation.json")
api CancelProjectAgentRun "$cancel" > "$scratch/cancel.json"
curl --fail --silent -X POST "http://127.0.0.1:$stub_port/fixture/release" >/dev/null
wait_state '.conversation.plannerSession.state=="expired"'
jq -e --arg run "$run" --argjson count "$assistant_count" '(.conversation.runs|any(.id==$run and .state=="cancelled")) and ([.conversation.messages[]|select(.role=="assistant")]|length)==$count' "$scratch/conversation.json" >/dev/null
api CancelProjectAgentRun "$cancel" > "$scratch/cancel-replay.json"
api ListProjectConversationHistory "$(jq -nc --arg p "$project" --arg c "$conversation" '{projectId:$p,conversationId:$c,query:"synthetic planner question 1",limit:20}')" > "$scratch/history.json"
jq -e '.records|length>0' "$scratch/history.json" >/dev/null
printf '%s\n' 'Session fixture passed: real resident worker reuse and deduplication; generation fences; controller restore/expiry/rotation/cancellation; original history retained.'
