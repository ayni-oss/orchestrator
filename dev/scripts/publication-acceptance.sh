#!/usr/bin/env bash
set -euo pipefail
echo 'This combined acceptance profile requires retired browser coverage and is unavailable until UI acceptance is rebuilt.' >&2
exit 2

readonly dev_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$dev_directory/lib/common.sh"
source "$dev_directory/lib/images.sh"
source "$dev_directory/lib/m4-acceptance.sh"
export HELM_NO_PLUGINS=1
readonly context="$(dev_context)"
readonly namespace="$(dev_namespace)"
[[ ( $# -eq 1 || ( $# -eq 2 && "$2" == --m4 ) ) && "$1" == /* && ! -e "$1" ]] || {
  echo 'usage: dev/ctl acceptance ABSOLUTE_NEW_EVIDENCE_DIRECTORY [--publication-only|--m4]' >&2; exit 2;
}
[[ "$namespace" == ayni-orchestrator-acceptance-* ]] || { echo 'isolated acceptance namespace required' >&2; exit 2; }
readonly evidence="$1"
readonly complete_m4="${2:-}"
expected_creates=1
expected_pulls=1
profile=publication
[[ "$complete_m4" != --m4 ]] || profile=m4
mkdir -p "$evidence"
status=failed
fixture_forward_pid=
publisher_stopped=false
cleanup() {
  if [[ -n "$fixture_forward_pid" ]]; then kill "$fixture_forward_pid" 2>/dev/null || true; fi
  if [[ "$publisher_stopped" == true ]]; then
    kubectl --context "$context" --namespace "$namespace" scale deployment/task-publisher --replicas=1 >/dev/null 2>&1 || true
  fi
  printf '{"result":"%s","profile":"%s","real_git":true,"github_protocol_fixture":true,"live_github_authentication":false,"cleanup":"retained_namespace_and_all_authoritative_stores"}\n' "$status" "$profile" > "$evidence/outcome.json"
}
trap cleanup EXIT
# Run the existing complete task-quality recovery profile before extending it.
AYNI_ORCHESTRATOR_TASK_FENCE_ACCEPTANCE=$([[ "$complete_m4" == --m4 ]] && echo true || echo false) \
  "$dev_directory/scripts/acceptance.sh" "$evidence/quality" --quality-only
quality_runtime="${AYNI_ORCHESTRATOR_QUALITY_RUNTIME_IMAGE:-}"
if [[ -z "$quality_runtime" ]]; then
  quality_runtime="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["runtime_image"])' "$DEV_REPOSITORY_ROOT/.ayni/task-quality-runtime.json")"
fi

control_pod() {
  kubectl --context "$context" --namespace "$namespace" get pods \
    -l app.kubernetes.io/name=ayni-orchestrator-control-plane --sort-by=.metadata.creationTimestamp \
    -o jsonpath='{.items[-1:].metadata.name}'
}
prepare_task() {
  kubectl --context "$context" --namespace "$namespace" exec "$(control_pod)" --container control-plane -- \
    env "ORCHESTRATOR_TASK_QUALITY_RUNTIME_IMAGE=$quality_runtime" \
    /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 "$1"
}
command_stdin() {
  kubectl --context "$context" --namespace "$namespace" exec -i "$(control_pod)" --container control-plane -- \
    /usr/local/bin/workflow-acceptance --endpoint http://127.0.0.1:50051 "$1" < "$2"
}
prepare_task --prepare-publication-task > "$evidence/publication-task.json"
fixture_image="$(dev_build_image publication-fixture)"
dev_load_image "$fixture_image"
printf '%s\n' "$fixture_image" > "$evidence/fixture-image.txt"
dev_build_chart_dependencies
control_image="$(kubectl --context "$context" --namespace "$namespace" get deployment control-plane -o jsonpath='{.spec.template.spec.containers[0].image}')"
executor_image="$(kubectl --context "$context" --namespace "$namespace" get statefulset conversation-supervisor -o jsonpath='{.spec.template.spec.containers[0].image}')"
bind_scope() {
  local workspace
  workspace="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["scope"]["workspace"])' "$1")"
  [[ "$workspace" =~ ^[a-f0-9-]{36}$ ]] || { echo 'invalid frozen fixture workspace' >&2; exit 1; }
  helm upgrade --install "$DEV_RELEASE_NAME" "$DEV_CHART" --reset-then-reuse-values \
    --kube-context "$context" --namespace "$namespace" \
    --set-string "ayni-orchestrator.images.controlPlane=$control_image" \
    --set-string "ayni-orchestrator.images.conversationSupervisor=$executor_image" \
    --set ayni-orchestrator.taskExecution.fixtureEnabled=true \
    --set publicationFixture.enabled=true --set-string "publicationFixture.image=$fixture_image" \
    --set ayni-orchestrator.publication.enabled=true \
    --set-string "ayni-orchestrator.publication.workspaceId=$workspace" \
    --set-string ayni-orchestrator.publication.repository=https://github.com/fixture/repo.git \
    --set-string ayni-orchestrator.publication.githubSecretName=publication-fixture-github \
    --set ayni-orchestrator.publication.fixtureEnabled=true \
    --set-string "ayni-orchestrator.publication.fixtureApiUrl=http://publication-fixture.$namespace.svc:8080" \
    --set-string "ayni-orchestrator.publication.fixtureGitUrl=git://publication-fixture.$namespace.svc:9418/fixture/repo.git" \
    --wait --timeout=5m >> "$evidence/publication-binding.txt"
}
forward_fixture() {
  if [[ -n "$fixture_forward_pid" ]]; then
    kill "$fixture_forward_pid" 2>/dev/null || true
    wait "$fixture_forward_pid" 2>/dev/null || true
  fi
  fixture_port="$(python3 -c 'import socket; s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')"
  kubectl --context "$context" --namespace "$namespace" port-forward --address 127.0.0.1 \
    service/publication-fixture "$fixture_port:8080" > "$evidence/fixture-port-forward.txt" 2>&1 &
  fixture_forward_pid=$!
  python3 - "$fixture_port" <<'PY'
import sys,time,urllib.request
for attempt in range(100):
    try:
        urllib.request.urlopen(f'http://127.0.0.1:{sys.argv[1]}/healthz',timeout=1).close()
        break
    except OSError:
        if attempt==99: raise
        time.sleep(.1)
PY
}
fixture() {
  python3 - "$fixture_port" "$@" <<'PY'
import json,sys,urllib.request
port,path=sys.argv[1:3]
body=json.loads(sys.argv[3]) if len(sys.argv)>3 else None
request=urllib.request.Request(f'http://127.0.0.1:{port}{path}',data=None if body is None else json.dumps(body).encode(),headers={'Content-Type':'application/json'})
with urllib.request.urlopen(request,timeout=10) as response:
    print(json.dumps(json.load(response),sort_keys=True))
PY
}
bind_scope "$evidence/publication-task.json"
forward_fixture
fixture /fixture/state > "$evidence/fixture-baseline.json"
fixture /fixture/control '{"drop_create_response":1}' > "$evidence/response-loss-control.json"
command_stdin --start-publication-stdin "$evidence/publication-task.json" > "$evidence/publication.json"
command_stdin --inspect-publication-stdin "$evidence/publication.json" > "$evidence/publication-identity.json"
fixture /fixture/state > "$evidence/fixture-after-publication.json"
python3 "$dev_directory/lib/publication_accounting.py" \
  "$evidence/fixture-baseline.json" "$evidence/fixture-after-publication.json" \
  "$expected_creates" "$expected_pulls" "$evidence/publication-identity.json"
# Closing an existing external draft never authorizes another publication intent.
pr_number="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["pr_number"])' "$evidence/publication-identity.json")"
fixture "/fixture/pulls/$pr_number" '{"state":"closed","merged":true}' > "$evidence/closed-existing-pr.json"
command_stdin --verify-publication-stdin "$evidence/publication.json" > "$evidence/closed-existing-replay.txt"
prepare_task --prepare-publication-failed-quality-task > "$evidence/failed-quality-task.json"
command_stdin --reject-publication-quality-stdin "$evidence/failed-quality-task.json" > "$evidence/failed-quality-rejected.txt"

# Freeze a queued intent before the scoped publisher can claim it, then cancel.
prepare_task --prepare-publication-task > "$evidence/cancel-task.json"
command_stdin --start-publication-pending-stdin "$evidence/cancel-task.json" > "$evidence/cancel-publication.json"
command_stdin --cancel-publication-stdin "$evidence/cancel-publication.json" > "$evidence/cancel-result.txt"

# The operator console exercises real canonical publication admission/retry and
# cancellation against a managed-quality task outside the publisher's bound workspace.
prepare_task --prepare-publication-task > "$evidence/console-task.json"
"$dev_directory/scripts/task-console-acceptance.sh" "$evidence/console-task.json" "$evidence/console"

# The expected-absent branch lease cannot overwrite another remote head.
prepare_task --prepare-publication-task > "$evidence/collision-task.json"
bind_scope "$evidence/collision-task.json"
publisher_pod="$(kubectl --context "$context" --namespace "$namespace" get pod -l app.kubernetes.io/name=ayni-task-publisher -o jsonpath='{.items[0].metadata.name}')"
kubectl --context "$context" --namespace "$namespace" scale deployment/task-publisher --replicas=0 > "$evidence/collision-publisher-stop.txt"
publisher_stopped=true
kubectl --context "$context" --namespace "$namespace" wait --for=delete "pod/$publisher_pod" --timeout=90s >> "$evidence/collision-publisher-stop.txt"
command_stdin --start-publication-pending-stdin "$evidence/collision-task.json" > "$evidence/collision-publication.json"
command_stdin --inspect-publication-stdin "$evidence/collision-publication.json" > "$evidence/collision-selection.json"
collision="$(python3 -c 'import json,sys;print(json.dumps({"branch":json.load(open(sys.argv[1]))["branch"],"value":"base"}))' "$evidence/collision-selection.json")"
fixture /fixture/branch "$collision" > "$evidence/collision-original-ref.json"
kubectl --context "$context" --namespace "$namespace" scale deployment/task-publisher --replicas=1 > "$evidence/collision-publisher-start.txt"
publisher_stopped=false
command_stdin --verify-publication-blocked-stdin "$evidence/collision-publication.json" > "$evidence/collision-blocked.txt"
command_stdin --inspect-publication-stdin "$evidence/collision-publication.json" > "$evidence/collision-final.json"
python3 - "$evidence/collision-final.json" <<'PY'
import json,sys
assert json.load(open(sys.argv[1]))['safe_reason']=='publication_ref_conflict'
PY

if [[ "$complete_m4" == --m4 ]]; then m4_publication_faults; fi

for workload in deployment/control-plane deployment/task-publisher deployment/publication-fixture deployment/task-supervisor statefulset/conversation-supervisor statefulset/jetstream statefulset/postgres; do
  name="${workload#*/}"
  kubectl --context "$context" --namespace "$namespace" rollout restart "$workload" > "$evidence/restart-$name.txt"
  kubectl --context "$context" --namespace "$namespace" rollout status "$workload" --timeout=3m >> "$evidence/restart-$name.txt"
  command_stdin --verify-publication-stdin "$evidence/publication.json" > "$evidence/replay-$name.txt"
  command_stdin --verify-publication-cancelled-stdin "$evidence/cancel-publication.json" > "$evidence/cancel-replay-$name.txt"
  if [[ "$name" == publication-fixture ]]; then forward_fixture; fi
  if [[ "$complete_m4" == --m4 ]]; then m4_verify_publications "$name"; fi
  fixture /fixture/state > "$evidence/fixture-after-$name.json"
  python3 "$dev_directory/lib/publication_accounting.py" \
    "$evidence/fixture-baseline.json" "$evidence/fixture-after-$name.json" \
    "$expected_creates" "$expected_pulls"
done
if [[ "$complete_m4" == --m4 ]]; then
  m4_retained_cleanup
  python3 "$dev_directory/scripts/summarize-m4.py" "$evidence" "$DEV_REPOSITORY_ROOT"
  python3 "$dev_directory/lib/publication_accounting.py" \
    "$evidence/fixture-baseline.json" "$evidence/cleanup-remote.json" \
    "$expected_creates" "$expected_pulls"
fi
status=passed
printf 'publication acceptance passed; real Git transport and GitHub protocol fixture; live GitHub authentication untested; evidence=%s\n' "$evidence"
