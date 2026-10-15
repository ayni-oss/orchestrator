#!/usr/bin/env bash
# Plan review/replay and schema migration on Docker Desktop's local Helm topology.
set -euo pipefail
export HELM_NO_PLUGINS=1
[[ $# == 2 ]] || { echo 'usage: dev/ctl test plans CONTROL_PLANE_IMAGE WEB_IMAGE' >&2; exit 2; }
root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
source "$root/dev/lib/common.sh"
namespace="ayni-plans-$$"
scratch=$(mktemp -d /tmp/ayni-plans.XXXXXX)
created=false
forward=''
port=${ORCHESTRATOR_PLANS_TEST_PORT:-19563}
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
    helm uninstall plan-validation --kube-context docker-desktop -n "$namespace" --ignore-not-found >/dev/null || status=1
    kubectl --context docker-desktop delete namespace "$namespace" --wait=false >/dev/null || status=1
    kubectl --context docker-desktop wait --for=delete "namespace/$namespace" --timeout=60s >/dev/null || status=1
  fi
  rm -rf "$scratch"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
kubectl --context docker-desktop get nodes >/dev/null
dev_build_chart_dependencies
AYNI_PLAN_FIXTURE_PATH="$scratch/current.sqlite" AYNI_LEGACY_PLAN_FIXTURE_PATH="$scratch/legacy.sqlite" \
  cargo test --locked -p ayni-orchestrator-control-plane export_project_plan_fixture -- --ignored
if kubectl --context docker-desktop get namespace "$namespace" >/dev/null 2>&1; then echo 'fixture namespace already exists' >&2; exit 1; fi
kubectl --context docker-desktop create namespace "$namespace"
created=true
kubectl --context docker-desktop label namespace "$namespace" orchestrator.ayni.dev/environment=local
helm upgrade --install plan-validation "$DEV_CHART" \
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
  k port-forward --address=127.0.0.1 service/control-plane "$port:50051" >"$scratch/forward.log" 2>&1 &
  forward=$!
  for _ in {1..30}; do
    if grpcurl -plaintext -max-time 2 "127.0.0.1:$port" list >/dev/null 2>&1; then return; fi
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
metadata: {name: plan-fixture}
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
k wait --for=condition=Ready pod/plan-fixture --timeout=120s
seed() {
  k exec plan-fixture -- node -e 'const f=require("fs"); for(const n of ["orchestrator.sqlite-wal","orchestrator.sqlite-shm"]) f.rmSync("/state/"+n,{force:true});'
  k exec -i plan-fixture -- node -e 'const f=require("fs");const s=f.createWriteStream("/state/orchestrator.sqlite");process.stdin.pipe(s);' < "$1"
}
# Assertions below use exporter-defined conversation IDs and public ProtoJSON.
inspect() {
  k exec plan-fixture -- node -e 'process.stdout.write(require("fs").readFileSync("/state/orchestrator.sqlite"));' > "$scratch/observed.sqlite"
  python3 "$root/dev/tests/inspect-plans.py" "$1" "$scratch/observed.sqlite"
}
manifest="$scratch/current.sqlite.json"
conversation=$(jq -r '.conversationId' "$manifest")
project=$(jq -r '.projectId' "$manifest")
plan=$(jq -r '.planId' "$manifest")
pending=$(jq -r '.pendingVersion' "$manifest")
approved=$(jq -r '.approvedVersion' "$manifest")
superseded=$(jq -r '.supersededVersion' "$manifest")
get() { api GetProjectConversation "$(jq -nc --arg id "$conversation" '{conversationId:$id}')"; }
decision() { jq -nc --arg c "$conversation" --arg p "$plan" --arg key "$1" --arg v "$2" --arg d "$3" '{idempotencyKey:$key,conversationId:$c,planId:$p,version:$v,decision:$d}'; }
for mode in current legacy; do
  seed "$scratch/$mode.sqlite"
  start
  get >"$scratch/$mode-initial.json"
  task_request=$(jq -nc --arg id "$(jq -r '.approvedTaskId' "$manifest")" '{developmentTaskId:$id}')
  grpcurl -plaintext -max-time 10 -d "$task_request" "127.0.0.1:$port" ayni.orchestrator.v1.DevelopmentTaskService/GetDevelopmentTask >"$scratch/$mode-task.json"
  jq -e --slurpfile c "$scratch/$mode-initial.json" '.developmentTask.snapshot.request==$c[0].conversation.proposals[0].request.request and .developmentTask.snapshot.repository.revision==$c[0].conversation.proposals[0].request.repositoryRevision and .developmentTask.snapshot.runtime.resolvedDigest==$c[0].conversation.proposals[0].researchedRuntime.resolvedDigest' "$scratch/$mode-task.json" >/dev/null
  jq -e '.conversation.plan.versions|length==3' "$scratch/$mode-initial.json" >/dev/null
  api ListProjectConversations "$(jq -nc --arg p "$project" '{projectId:$p}')" >"$scratch/$mode-list.json"
  jq -e --arg id "$conversation" '.conversations|any(.id==$id)' "$scratch/$mode-list.json" >/dev/null
  # Replays retain the admitted task identity without calling GitHub or a model.
  api DecideProjectPlan "$(decision fixture-approve1 "$approved" approve)" >"$scratch/$mode-approved.json"
  api DecideProjectPlan "$(decision "different-approve-$mode" "$approved" approve)" >"$scratch/$mode-duplicate.json"
  cmp "$scratch/$mode-approved.json" "$scratch/$mode-duplicate.json"
  api DecideProjectProposal "$(jq -nc --arg c "$conversation" --arg p "$(jq -r '.legacyProposalId' "$manifest")" '{idempotencyKey:"fixture-legacy-replay",conversationId:$c,proposalId:$p,decision:"approve"}')" >"$scratch/$mode-legacy-replay.json"
  cmp "$scratch/$mode-approved.json" "$scratch/$mode-legacy-replay.json"
  reject DecideProjectPlan "$(decision "stale-$mode" "$superseded" reject)" Aborted
  pending_proposal=$(jq -r '.conversation.plan.versions[-1].proposalIds[0]' "$scratch/$mode-initial.json")
  reject DecideProjectProposal "$(jq -nc --arg c "$conversation" --arg p "$pending_proposal" --arg mode "$mode" '{idempotencyKey:("legacy-pending-"+$mode),conversationId:$c,proposalId:$p,decision:"reject"}')" FailedPrecondition
  # Race two mutually exclusive reviews; one must win and the other must conflict.
  api DecideProjectPlan "$(decision "reject-$mode" "$pending" reject)" >"$scratch/race-reject.json" 2>"$scratch/race-reject.err" &
  reject_pid=$!
  api DecideProjectPlan "$(decision "revise-$mode" "$pending" revise)" >"$scratch/race-revise.json" 2>"$scratch/race-revise.err" &
  revise_pid=$!
  reject_status=0; wait "$reject_pid" || reject_status=$?
  revise_status=0; wait "$revise_pid" || revise_status=$?
  [[ ( "$reject_status" == 0 && "$revise_status" != 0 ) || ( "$reject_status" != 0 && "$revise_status" == 0 ) ]] || { echo 'concurrent review did not produce exactly one winner' >&2; exit 1; }
  if [[ "$reject_status" == 0 ]]; then
    winner=reject
    rg -q 'Code: FailedPrecondition' "$scratch/race-revise.err"
  else
    winner=revise
    rg -q 'Code: FailedPrecondition' "$scratch/race-reject.err"
  fi
  get >"$scratch/$mode-decided.json"
  jq -e --arg state "$winner" '.conversation.plan.versions[-1].state==($state|if .=="reject" then "rejected" else . end)' "$scratch/$mode-decided.json" >/dev/null
  stop
  inspect "$scratch/$mode.sqlite"
  start
  get >"$scratch/$mode-restarted.json"
  cmp "$scratch/$mode-decided.json" "$scratch/$mode-restarted.json"
  grpcurl -plaintext -max-time 10 -d "$task_request" "127.0.0.1:$port" ayni.orchestrator.v1.DevelopmentTaskService/GetDevelopmentTask >"$scratch/$mode-restarted-task.json"
  cmp "$scratch/$mode-task.json" "$scratch/$mode-restarted-task.json"
  api DecideProjectPlan "$(decision "$winner-$mode" "$pending" "$winner")" >"$scratch/$mode-replayed-decision.json"
  cmp "$scratch/$mode-decided.json" "$scratch/$mode-replayed-decision.json"
  api DecideProjectPlan "$(decision fixture-approve1 "$approved" approve)" >"$scratch/$mode-restarted-approval.json"
  cmp "$scratch/$mode-decided.json" "$scratch/$mode-restarted-approval.json"
  api ListProjectConversations "$(jq -nc --arg p "$project" '{projectId:$p}')" >"$scratch/$mode-restarted-list.json"
  jq -e --arg id "$conversation" '.conversations|any(.id==$id)' "$scratch/$mode-restarted-list.json" >/dev/null
  stop
  inspect "$scratch/$mode.sqlite"
done
echo 'Plan history, approved-task replay, stale/concurrent review, schema migration and restart persistence passed (no provider or worker execution).'
