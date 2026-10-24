#!/usr/bin/env bash
set -euo pipefail
readonly dev_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$dev_directory/lib/common.sh"
export HELM_NO_PLUGINS=1
readonly context="$(dev_context)" namespace="$(dev_namespace)"
[[ $# -eq 1 && "$1" == /* && ! -e "$1" && "$namespace" == ayni-orchestrator-acceptance-* ]] || {
  echo 'usage: dev/ctl acceptance ABSOLUTE_NEW_EVIDENCE_DIRECTORY --m8 (owned acceptance namespace required)' >&2; exit 2;
}
readonly evidence="$1"
mkdir -p "$evidence"
status=failed
supervisor_stopped=false
cleanup() {
  if [[ "$supervisor_stopped" == true ]]; then
    kubectl --context "$context" --namespace "$namespace" scale deployment/task-supervisor --replicas=1 >/dev/null 2>&1 || true
  fi
  printf '{"result":"%s","profile":"operational-acceptance","milestone_complete":false,"cleanup":"retained_authoritative_stores"}\n' "$status" > "$evidence/outcome.json"
}
trap cleanup EXIT
cd "$DEV_REPOSITORY_ROOT"
# Preserve the established deployed failure gates and extend them with distinct
# concurrent capacity and dependency admission evidence. These subreports keep
# their own fixture boundaries and unmet prerequisites.
"$dev_directory/scripts/publication-acceptance.sh" "$evidence/m4" --m4
"$dev_directory/scripts/recovery-acceptance.sh" "$evidence/m6"
control_pod() {
  kubectl --context "$context" --namespace "$namespace" get pods \
    -l app.kubernetes.io/name=ayni-orchestrator-control-plane --sort-by=.metadata.creationTimestamp -o jsonpath='{.items[-1:].metadata.name}'
}
command_stdin() {
  kubectl --context "$context" --namespace "$namespace" exec -i "$(control_pod)" --container control-plane -- \
    /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 "$1" < "$2"
}
kubectl --context "$context" --namespace "$namespace" scale deployment/task-supervisor --replicas=0 > "$evidence/supervisor-stop.txt"
supervisor_stopped=true
kubectl --context "$context" --namespace "$namespace" wait --for=delete pod \
  -l app.kubernetes.io/name=ayni-task-supervisor --timeout=2m >> "$evidence/supervisor-stop.txt"
kubectl --context "$context" --namespace "$namespace" exec "$(control_pod)" --container control-plane -- \
  /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 --prepare-operational-concurrency > "$evidence/concurrency-queued.json"
kubectl --context "$context" --namespace "$namespace" scale deployment/task-supervisor --replicas=1 > "$evidence/supervisor-start.txt"
kubectl --context "$context" --namespace "$namespace" rollout status deployment/task-supervisor --timeout=2m >> "$evidence/supervisor-start.txt"
supervisor_stopped=false
command_stdin --capture-operational-concurrency-stdin "$evidence/concurrency-queued.json" > "$evidence/concurrency-active.json"
kubectl --context "$context" --namespace "${namespace}-sessions" get pod -l app.kubernetes.io/name=ayni-task-worker -o json > "$evidence/concurrency-pods.json"
kubectl --context "$context" --namespace "$namespace" rollout restart deployment/task-supervisor > "$evidence/concurrency-adoption.txt"
kubectl --context "$context" --namespace "$namespace" rollout status deployment/task-supervisor --timeout=2m >> "$evidence/concurrency-adoption.txt"
command_stdin --capture-operational-concurrency-stdin "$evidence/concurrency-active.json" > "$evidence/concurrency-adopted.json"
python3 - "$evidence/concurrency-active.json" "$evidence/concurrency-adopted.json" <<'PY'
import json,sys
before,after=[json.load(open(path)) for path in sys.argv[1:]]
assert before['active']==after['active'], 'supervisor restart changed active authority or Pod identity'
PY
command_stdin --cancel-operational-concurrency-stdin "$evidence/concurrency-adopted.json" > "$evidence/concurrency-terminal.json"
for workload in deployment/control-plane deployment/task-supervisor statefulset/workflow-postgres; do
  name="${workload#*/}"
  kubectl --context "$context" --namespace "$namespace" rollout restart "$workload" > "$evidence/restart-$name.txt"
  kubectl --context "$context" --namespace "$namespace" rollout status "$workload" --timeout=3m >> "$evidence/restart-$name.txt"
  command_stdin --verify-operational-concurrency-stdin "$evidence/concurrency-terminal.json" > "$evidence/concurrency-replay-$name.txt"
done
"$dev_directory/scripts/check.sh" > "$evidence/operational-readiness.txt" 2>&1
kubectl --context "$context" --namespace "$namespace" exec "$(control_pod)" --container control-plane -- \
  /usr/local/bin/workflow-diagnostic --endpoint http://127.0.0.1:50051 > "$evidence/conversation-diagnostic.txt"
# These separate disposable-database gates exercise source retirement safety,
# multi-upstream selection, questions/answers and budgeted assessments. They are
# integration evidence, not accelerated thirty-day production retention proof.
scripts/ci/check-workflow-postgres-feasibility.sh > "$evidence/postgres-integration.txt" 2>&1
if [[ -n "${AYNI_ORCHESTRATOR_SELF_DEVELOPMENT_RUNTIME_IMAGE:-}" ]]; then
  "$dev_directory/scripts/self-development-acceptance.sh" "$evidence/self-development" "$AYNI_ORCHESTRATOR_SELF_DEVELOPMENT_RUNTIME_IMAGE"
fi
python3 "$dev_directory/lib/operational-report.py" "$evidence"
status=passed
printf 'Operational acceptance gates passed; see report for unresolved closure gates. evidence=%s\n' "$evidence"
