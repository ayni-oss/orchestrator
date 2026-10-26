#!/usr/bin/env bash
set -euo pipefail
echo 'This combined acceptance profile requires retired browser coverage and is unavailable until UI acceptance is rebuilt.' >&2
exit 2

readonly dev_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$dev_directory/lib/common.sh"
export HELM_NO_PLUGINS=1
readonly context="$(dev_context)" namespace="$(dev_namespace)"
[[ $# -eq 1 && "$1" == /* && ! -e "$1" && "$namespace" == ayni-orchestrator-acceptance-* ]] || {
  echo 'usage: dev/ctl acceptance ABSOLUTE_NEW_EVIDENCE_DIRECTORY --m8 (owned acceptance namespace required)' >&2; exit 2;
}
readonly evidence="$1"
cd "$DEV_REPOSITORY_ROOT"
[[ -z "$(git status --porcelain --untracked-files=all)" ]] || {
  echo 'M8 requires a clean committed checkout, including tracked and untracked source, before freezing evidence' >&2; exit 2;
}
runtime="${AYNI_ORCHESTRATOR_SELF_DEVELOPMENT_RUNTIME_IMAGE:-}"
[[ "$runtime" =~ @sha256:[a-f0-9]{64}$ ]] || {
  echo 'M8 requires AYNI_ORCHESTRATOR_SELF_DEVELOPMENT_RUNTIME_IMAGE with the digest-pinned own-source managed runtime' >&2; exit 2;
}
quality_runtime="${AYNI_ORCHESTRATOR_QUALITY_RUNTIME_IMAGE:-}"
if [[ -z "$quality_runtime" ]]; then
  quality_runtime="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["runtime_image"])' "$DEV_REPOSITORY_ROOT/.ayni/task-quality-runtime.json")"
fi
[[ "$quality_runtime" =~ @sha256:[a-f0-9]{64}$ ]] || { echo 'M8 requires the digest-pinned task-quality fixture runtime' >&2; exit 2; }
export AYNI_ORCHESTRATOR_QUALITY_RUNTIME_IMAGE="$quality_runtime"
cd "$DEV_REPOSITORY_ROOT"
node --input-type=module -e "import { chromium } from 'playwright'; import { existsSync } from 'node:fs'; if (!existsSync(chromium.executablePath())) throw Error('install Playwright Chromium before M8 acceptance');"
mkdir -p "$evidence"
status=failed
supervisor_stopped=false
metrics_pid=
cleanup() {
  [[ -z "$metrics_pid" ]] || kill "$metrics_pid" 2>/dev/null || true
  if [[ "$supervisor_stopped" == true ]]; then
    kubectl --context "$context" --namespace "$namespace" scale deployment/task-supervisor --replicas=1 >/dev/null 2>&1 || true
  fi
  printf '{"result":"%s","profile":"operational-acceptance","milestone_complete":false,"local_acceptance_complete":false,"cleanup":"retained_authoritative_stores"}\n' "$status" > "$evidence/outcome.json"
}
trap cleanup EXIT
cd "$DEV_REPOSITORY_ROOT"
git rev-parse HEAD > "$evidence/source-revision.txt"
python3 - "$evidence/source-identity.json" <<'PY_SOURCE'
import hashlib,json,pathlib,subprocess,sys
paths=subprocess.check_output(['git','ls-files','--cached','--others','--exclude-standard','-z']).decode().split('\0')
manifest={p:hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest() for p in paths if p and pathlib.Path(p).is_file()}
json.dump({'revision':subprocess.check_output(['git','rev-parse','HEAD']).decode().strip(), 'lock_sha256':hashlib.sha256(pathlib.Path('.ayni.lock').read_bytes()).hexdigest(), 'manifest':manifest},open(sys.argv[1],'x'),sort_keys=True,indent=2)
PY_SOURCE
# Capture only hardware, resource and immutable workload identity projections.
# Environment values, Secret bodies and transcripts never enter this inventory.
kubectl --context "$context" get nodes -o json | python3 -c 'import json,sys; print(json.dumps([{ "name": n["metadata"]["name"], "capacity":n["status"]["capacity"], "allocatable":n["status"]["allocatable"], "system":n["status"]["nodeInfo"]} for n in json.load(sys.stdin)["items"]],sort_keys=True))' > "$evidence/hardware.json"
for scope in "$namespace" "${namespace}-sessions"; do
  kubectl --context "$context" --namespace "$scope" get pods -o json | python3 -c 'import json,sys; print(json.dumps([{ "name":p["metadata"]["name"], "uid":p["metadata"]["uid"], "containers":[{"name":c["name"],"image":c["image"],"resources":c.get("resources",{})} for c in p["spec"]["containers"]],"image_ids":[{"name":c["name"],"image_id":c.get("imageID","")} for c in p.get("status",{}).get("containerStatuses",[])]} for p in json.load(sys.stdin)["items"]],sort_keys=True))' > "$evidence/workloads-$scope.json"
done
scripts/ci/check-workflow-private-v1-feasibility.sh > "$evidence/private-v1-cutover.txt" 2>&1
# Preserve the established deployed failure gates and extend them with distinct
# concurrent capacity and dependency admission evidence. These subreports keep
# their own fixture boundaries and unmet prerequisites.
# Provider fixtures are not Helm-owned. Stop this known local fixture before
# cumulative cleanup waits for all parent Pods; keep its Deployment and stores.
fixture_deployment="$(kubectl --context "$context" --namespace "$namespace" get deployment provider-fixture --ignore-not-found -o json)"
if [[ -n "$fixture_deployment" ]]; then
  python3 -c 'import json,sys; d=json.load(sys.stdin); assert d["spec"]["selector"]["matchLabels"]=={"app":"provider-fixture"}; assert [c["name"] for c in d["spec"]["template"]["spec"]["containers"]]==["fixture"], "refusing to stop an unrelated Deployment"' <<< "$fixture_deployment"
  kubectl --context "$context" --namespace "$namespace" scale deployment/provider-fixture --replicas=0 > "$evidence/provider-fixture-stop.txt"
  kubectl --context "$context" --namespace "$namespace" wait --for=delete pod -l app=provider-fixture --timeout=2m >> "$evidence/provider-fixture-stop.txt"
fi
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
metrics_port="$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')"
kubectl --context "$context" --namespace "$namespace" port-forward --address 127.0.0.1 "pod/$(control_pod)" "$metrics_port:8080" > "$evidence/metrics-forward.txt" 2>&1 &
metrics_pid=$!
python3 - "$evidence" "$metrics_port" <<'PY_METRICS'
import pathlib,sys,time,urllib.request
root=pathlib.Path(sys.argv[1]); endpoint=f'http://127.0.0.1:{sys.argv[2]}'
for attempt in range(100):
    try:
        ready=urllib.request.urlopen(endpoint+'/readyz',timeout=2).read().decode()
        assert ready.strip()=='ready', 'control plane not ready'
        break
    except OSError:
        if attempt==99: raise
        time.sleep(.1)
(root/'readyz.txt').write_text(ready)
(root/'metrics.txt').write_bytes(urllib.request.urlopen(endpoint+'/metrics',timeout=5).read())
PY_METRICS
kill "$metrics_pid"; wait "$metrics_pid" 2>/dev/null || true; metrics_pid=''

kubectl --context "$context" --namespace "$namespace" exec "$(control_pod)" --container control-plane -- \
  /usr/local/bin/workflow-diagnostic --endpoint http://127.0.0.1:50051 > "$evidence/conversation-diagnostic.txt"
# These separate disposable-database gates exercise source retirement safety,
# multi-upstream selection, questions/answers and budgeted assessments. They are
# integration evidence, including accelerated persisted-clock retention expiry.
scripts/ci/check-workflow-postgres-feasibility.sh > "$evidence/postgres-integration.txt" 2>&1
"$dev_directory/scripts/dependency-acceptance.sh" "$evidence/dependencies"
"$dev_directory/scripts/assessment-acceptance.sh" "$evidence/assessment"
"$dev_directory/scripts/worker-recovery-acceptance.sh" "$evidence/worker-recovery"
executor_image="$(kubectl --context "$context" --namespace "$namespace" get statefulset scripted-executor -o jsonpath='{.spec.template.spec.containers[0].image}')"
for provider in openai anthropic; do
  read -r worker_port fixture_port < <(python3 - <<'PY_PORTS'
import socket
sockets=[socket.socket() for _ in range(2)]
for sock in sockets: sock.bind(('127.0.0.1',0))
print(*(sock.getsockname()[1] for sock in sockets))
for sock in sockets: sock.close()
PY_PORTS
)
  MCP_WORKER_PORT="$worker_port" MCP_FIXTURE_PORT="$fixture_port" "$dev_directory/fixtures/mcp/acceptance.sh" "$executor_image" "$provider" "$evidence/mcp-$provider" > "$evidence/mcp-$provider.txt" 2>&1
done
"$dev_directory/scripts/self-development-acceptance.sh" "$evidence/self-development" "$runtime"
# A report cannot combine frozen source evidence with a later checkout.
python3 - "$evidence/source-identity.json" <<'PY_SOURCE'
import hashlib,json,pathlib,subprocess,sys
before=json.load(open(sys.argv[1]))
assert before['revision']==subprocess.check_output(['git','rev-parse','HEAD']).decode().strip(), 'source revision changed'
paths=subprocess.check_output(['git','ls-files','--cached','--others','--exclude-standard','-z']).decode().split('\0')
after={p:hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest() for p in paths if p and pathlib.Path(p).is_file()}
assert before['manifest']==after, 'source files changed during M8 acceptance'
PY_SOURCE
python3 "$dev_directory/lib/operational-report.py" "$evidence"
status=passed
python3 - "$evidence/acceptance.json" "$evidence/outcome.json" <<'PY_OUTCOME'
import json,sys
report=json.load(open(sys.argv[1]))
json.dump({'result':'passed','profile':'operational-acceptance','milestone_complete':report['milestone_complete'],'local_acceptance_complete':report['local_acceptance_complete'],'cleanup':'retained_authoritative_stores'},open(sys.argv[2],'w'),sort_keys=True)
PY_OUTCOME
trap - EXIT
printf 'Operational acceptance gates passed; see report for unresolved closure gates. evidence=%s\n' "$evidence"
