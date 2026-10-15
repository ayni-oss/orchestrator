#!/usr/bin/env bash
# Workspace/project ownership and restart persistence on the local Helm topology.
set -euo pipefail
export HELM_NO_PLUGINS=1
[[ $# == 2 ]] || { echo 'usage: dev/ctl test workspaces CONTROL_PLANE_IMAGE WEB_IMAGE' >&2; exit 2; }
root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
source "$root/dev/lib/common.sh"
namespace="ayni-workspaces-$$"
scratch=$(mktemp -d /tmp/ayni-workspaces.XXXXXX)
created=false
forward=''
port=${ORCHESTRATOR_WORKSPACES_TEST_PORT:-19562}
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
    helm uninstall workspace-validation --kube-context docker-desktop -n "$namespace" --ignore-not-found >/dev/null || status=1
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
if kubectl --context docker-desktop get namespace "$namespace" >/dev/null 2>&1; then echo 'fixture namespace already exists' >&2; exit 1; fi
kubectl --context docker-desktop create namespace "$namespace"
created=true
kubectl --context docker-desktop label namespace "$namespace" orchestrator.ayni.dev/environment=local
helm upgrade --install workspace-validation "$DEV_CHART" \
  --kube-context docker-desktop -n "$namespace" --wait --timeout=5m \
  --set observability.enabled=false --set ayni-orchestrator.runtime.enabled=false \
  --set-string "ayni-orchestrator.images.controlPlane=$1" --set-string "ayni-orchestrator.images.web=$2" \
  --set ayni-orchestrator.images.pullPolicy=Never
start_forward() {
  k port-forward --address=127.0.0.1 service/control-plane "$port:50051" >"$scratch/forward.log" 2>&1 &
  forward=$!
  for _ in {1..30}; do
    if grpcurl -plaintext -max-time 2 "127.0.0.1:$port" list >/dev/null 2>&1; then return; fi
    sleep 1
  done
  cat "$scratch/forward.log" >&2
  return 1
}
api() { grpcurl -plaintext -max-time 10 -d "$2" "127.0.0.1:$port" "ayni.orchestrator.v1.$1"; }
reject() {
  if api "$1" "$2" >"$scratch/rejected.json" 2>"$scratch/rejected-error"; then
    echo "unexpected successful admission: $1" >&2; exit 1
  fi
  rg -q "Code: $3" "$scratch/rejected-error" || { cat "$scratch/rejected-error" >&2; exit 1; }
}
start_forward
# Local metadata only: no credentials, provider calls or worker execution.
for number in 1 2; do
  api RepositoryService/CreateRepositoryConnection "{\"idempotencyKey\":\"workspace-repo-$number\",\"provider\":{\"github\":{\"repositoryId\":\"$number\",\"owner\":\"fixture\",\"name\":\"repository-$number\",\"installationId\":\"1\",\"htmlUrl\":\"https://github.com/fixture/repository-$number\"}},\"displayName\":\"Repository $number\",\"defaultBranch\":\"main\",\"useAyni\":false}" >"$scratch/repository-$number.json"
done
one=$(jq -r '.repositoryConnection.id' "$scratch/repository-1.json")
two=$(jq -r '.repositoryConnection.id' "$scratch/repository-2.json")
workspace_request=$(jq -nc --arg one "$one" '{idempotencyKey:"workspace-create",displayName:"Shared workspace",repositoryConnectionIds:[$one]}')
api ProjectService/CreateWorkspace "$workspace_request" >"$scratch/workspace.json"
workspace=$(jq -r '.workspace.id' "$scratch/workspace.json")
api ProjectService/CreateWorkspace "$workspace_request" >"$scratch/workspace-replay.json"
cmp "$scratch/workspace.json" "$scratch/workspace-replay.json"
reject ProjectService/CreateWorkspace "$(jq '.displayName="Conflicting request"' <<<"$workspace_request")" AlreadyExists
project_request=$(jq -nc --arg workspace "$workspace" --arg one "$one" '{idempotencyKey:"project-create",displayName:"Improve onboarding",goal:"Make onboarding reliable",workspaceId:$workspace,repositoryConnectionIds:[$one]}')
api ProjectService/CreateProject "$project_request" >"$scratch/project.json"
project=$(jq -r '.project.id' "$scratch/project.json")
jq -e --arg workspace "$workspace" '.project.workspaceId==$workspace and .project.goal=="Make onboarding reliable"' "$scratch/project.json" >/dev/null
reject ProjectService/CreateProject "$(jq -nc --arg workspace "$workspace" --arg two "$two" '{idempotencyKey:"outside-membership",displayName:"Invalid project",workspaceId:$workspace,repositoryConnectionIds:[$two]}')" FailedPrecondition
update_request=$(jq -nc --arg workspace "$workspace" --arg one "$one" --arg two "$two" '{idempotencyKey:"workspace-update",workspaceId:$workspace,expectedResourceVersion:"1",displayName:"Shared development",repositoryConnectionIds:[$one,$two]}')
api ProjectService/UpdateWorkspace "$update_request" >"$scratch/updated.json"
reject ProjectService/UpdateWorkspace "$(jq '.idempotencyKey="stale-update"' <<<"$update_request")" Aborted
# Removing shared access preserves historical project selections.
api ProjectService/UpdateWorkspace "$(jq -nc --arg workspace "$workspace" --arg two "$two" '{idempotencyKey:"remove-selected-member",workspaceId:$workspace,expectedResourceVersion:"2",displayName:"Shared development",repositoryConnectionIds:[$two]}')" >"$scratch/removed.json"
reject ProjectService/CreateProject "$(jq '.idempotencyKey="removed-member-project"' <<<"$project_request")" FailedPrecondition
reject ProjectConversationService/CreateProjectConversation "$(jq -nc --arg project "$project" --arg one "$one" '{idempotencyKey:"removed-member-research",projectId:$project,title:"Research removed member",repositoryConnectionIds:[$one]}')" FailedPrecondition
kill "$forward" 2>/dev/null || true; wait "$forward" 2>/dev/null || true; forward=''
k rollout restart deployment/control-plane
k rollout status deployment/control-plane --timeout=180s
start_forward
api ProjectService/GetWorkspace "{\"workspaceId\":\"$workspace\"}" >"$scratch/restarted-workspace.json"
api ProjectService/GetProject "{\"projectId\":\"$project\"}" >"$scratch/restarted-project.json"
cmp "$scratch/removed.json" "$scratch/restarted-workspace.json"
cmp "$scratch/project.json" "$scratch/restarted-project.json"
api ProjectService/CreateProject "$project_request" >"$scratch/restarted-replay.json"
cmp "$scratch/project.json" "$scratch/restarted-replay.json"
api ProjectService/ListWorkspaces '{}' >"$scratch/list.json"
jq -e --arg id "$workspace" '.workspaces|length==1 and .[0].id==$id' "$scratch/list.json" >/dev/null
archive_request=$(jq -nc --arg workspace "$workspace" '{idempotencyKey:"workspace-archive",workspaceId:$workspace,expectedResourceVersion:"3"}')
api ProjectService/ArchiveWorkspace "$archive_request" >"$scratch/archived.json"
jq -e '.workspace.state=="WORKSPACE_STATE_ARCHIVED"' "$scratch/archived.json" >/dev/null
reject ProjectService/CreateProject "$(jq --arg two "$two" '.idempotencyKey="after-archive" | .repositoryConnectionIds=[$two]' <<<"$project_request")" FailedPrecondition
# Archiving gates new work while retaining project history.
api ProjectService/GetProject "{\"projectId\":\"$project\"}" >"$scratch/retained.json"
cmp "$scratch/project.json" "$scratch/retained.json"
kill "$forward" 2>/dev/null || true; wait "$forward" 2>/dev/null || true; forward=''
k rollout restart deployment/control-plane
k rollout status deployment/control-plane --timeout=180s
start_forward
api ProjectService/GetWorkspace "{\"workspaceId\":\"$workspace\"}" >"$scratch/restarted-archive.json"
cmp "$scratch/archived.json" "$scratch/restarted-archive.json"
echo 'Workspace ownership, optimistic concurrency, idempotent replay, restart persistence and archive admission passed.'
