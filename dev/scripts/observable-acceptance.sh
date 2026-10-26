#!/usr/bin/env bash
set -euo pipefail
readonly dev_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$dev_directory/lib/common.sh"
readonly context="$(dev_context)" namespace="$(dev_namespace)"
[[ $# -eq 1 && "$1" == /* && ! -e "$1" ]] || {
  echo 'usage: dev/ctl acceptance ABSOLUTE_NEW_EVIDENCE_DIRECTORY --observable' >&2; exit 2;
}
[[ "$namespace" == ayni-orchestrator-acceptance-* ]] || {
  echo 'observable acceptance requires an isolated ayni-orchestrator-acceptance-* namespace' >&2; exit 2;
}
cd "$DEV_REPOSITORY_ROOT"
[[ -z "$(git status --porcelain)" ]] || {
  echo 'observable acceptance requires committed source with a clean working tree' >&2; exit 2;
}
kubectl --context "$context" --namespace "$namespace" rollout status deployment/workflow-traces --timeout=60s
for workload in deployment/control-plane statefulset/scripted-executor; do
  image="$(kubectl --context "$context" --namespace "$namespace" get "$workload" -o jsonpath='{.spec.template.spec.containers[0].image}')"
  read -r revision dirty < <(docker image inspect "$image" --format '{{index .Config.Labels "org.opencontainers.image.revision"}} {{index .Config.Labels "org.opencontainers.image.source.dirty"}}')
  [[ "$revision" == "$(git rev-parse HEAD)" && "$dirty" == false ]] || {
    echo 'observable acceptance requires both workload images built from this clean committed revision' >&2; exit 2;
  }
done
readonly evidence="$1"
mkdir -p "$evidence"
status=failed
console_pid=
forward_pid=
cleanup() {
  if [[ -n "$console_pid" ]]; then kill "$console_pid" 2>/dev/null || true; wait "$console_pid" 2>/dev/null || true; fi
  if [[ -n "$forward_pid" ]]; then kill "$forward_pid" 2>/dev/null || true; wait "$forward_pid" 2>/dev/null || true; fi
  python3 - "$evidence" "$status" "$namespace" <<'PY'
import json,pathlib,sys
root=pathlib.Path(sys.argv[1])
root.joinpath('outcome.json').write_text(json.dumps({
    'result':sys.argv[2], 'profile':'observable-execution', 'namespace':sys.argv[3],
    'provider':'scripted', 'live_provider_qualification':False,
    'publication':'real Git with local GitHub protocol fixture',
    'cleanup':'retained namespace and all authoritative stores',
},indent=2)+'\n')
PY
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
git rev-parse HEAD > "$evidence/source-revision.txt"
git ls-files -z | python3 -c '
import hashlib,json,pathlib,sys
root=pathlib.Path(sys.argv[1])
manifest={name:hashlib.sha256(root.joinpath(name).read_bytes()).hexdigest()
for name in sys.stdin.buffer.read().decode().split("\0") if name and root.joinpath(name).is_file()}
print(json.dumps(manifest,sort_keys=True,indent=2))
' "$DEV_REPOSITORY_ROOT" > "$evidence/source-manifest.json"
# Includes isolated coding, managed quality, cancellation, Pod loss, retained
# restart recovery and bounded draft publication against the local Git fixture.
"$dev_directory/scripts/publication-acceptance.sh" "$evidence/tasks" > "$evidence/tasks.log" 2>&1
pnpm web:build > "$evidence/web-build.log" 2>&1
cargo build --locked -p ayni-orchestrator-control-plane --bin workflow-console > "$evidence/gateway-build.log" 2>&1
read -r port upstream < <(python3 - <<'PY'
import socket
a,b=socket.socket(),socket.socket()
a.bind(('127.0.0.1',0)); b.bind(('127.0.0.1',0))
print(a.getsockname()[1],b.getsockname()[1]);a.close();b.close()
PY
)
pod="$(kubectl --context "$context" --namespace "$namespace" get pods \
  -l app.kubernetes.io/name=ayni-orchestrator-control-plane --sort-by=.metadata.creationTimestamp \
  -o jsonpath='{.items[-1:].metadata.name}')"
kubectl --context "$context" --namespace "$namespace" port-forward --address 127.0.0.1 \
  "pod/$pod" "$upstream:50051" > "$evidence/port-forward.log" 2>&1 &
forward_pid=$!
target/debug/workflow-console "127.0.0.1:$port" "127.0.0.1:$upstream" "$DEV_REPOSITORY_ROOT/apps/web/build" > "$evidence/gateway.log" 2>&1 &
console_pid=$!
python3 - "$port" "$console_pid" "$upstream" <<'PY'
import os,socket,sys,time,urllib.request
for attempt in range(100):
    os.kill(int(sys.argv[2]),0)
    try:
        with socket.create_connection(('127.0.0.1',int(sys.argv[3])),timeout=1):pass
        urllib.request.urlopen(f'http://127.0.0.1:{sys.argv[1]}/workspace/',timeout=1).close()
        break
    except OSError:
        if attempt==99: raise
        time.sleep(.1)
PY
task_scope="$(python3 - "$evidence/tasks/publication-task.json" <<'PY'
import json,sys
s=json.load(open(sys.argv[1]))['scope']
print(json.dumps({'workspaceId':s['workspace'],'projectId':s['project'],'conversationId':s['conversation']}))
PY
)"
WEB_URL="http://127.0.0.1:$port/workspace/" WEB_EVIDENCE_DIRECTORY="$evidence/browser" \
  WEB_TASK_SCOPE="$task_scope" node apps/web/test/browser.mjs > "$evidence/browser.log" 2>&1
python3 scripts/ci/check-execution-traces.py --namespace "$namespace" --output "$evidence/traces" --profile full > "$evidence/traces.log" 2>&1
"$dev_directory/scripts/check.sh" > "$evidence/readiness.txt" 2>&1
[[ -z "$(git status --porcelain)" ]] || {
  echo 'source changed during observable acceptance' >&2; exit 1;
}
[[ "$(git rev-parse HEAD)" == "$(cat "$evidence/source-revision.txt")" ]] || {
  echo 'source revision changed during observable acceptance' >&2; exit 1;
}
python3 - "$evidence/source-manifest.json" "$DEV_REPOSITORY_ROOT" <<'PY'
import hashlib,json,pathlib,sys
root=pathlib.Path(sys.argv[2])
for name,expected in json.load(open(sys.argv[1])).items():
    if hashlib.sha256(root.joinpath(name).read_bytes()).hexdigest()!=expected:
        raise SystemExit('source fingerprint changed during observable acceptance')
PY
status=passed
printf 'observable execution acceptance passed; scripted conversations and managed task/publication fixtures; evidence=%s\n' "$evidence"
