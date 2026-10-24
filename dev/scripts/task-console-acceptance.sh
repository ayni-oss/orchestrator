#!/usr/bin/env bash
set -euo pipefail
readonly dev_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$dev_directory/lib/common.sh"
[[ ( $# -eq 2 || ( $# -eq 3 && ( "$3" == --recovery || "$3" == --continuation || "$3" == --fresh-continuation ) ) ) && "$1" == /* && "$2" == /* && ! -e "$2" ]] || {
  echo 'usage: task-console-acceptance.sh ABSOLUTE_TASK_EVIDENCE_JSON ABSOLUTE_NEW_EVIDENCE_DIRECTORY [--recovery|--continuation|--fresh-continuation]' >&2; exit 2;
}
selection="$1"
readonly evidence="$2" context="$(dev_context)" namespace="$(dev_namespace)"
[[ "$namespace" == ayni-orchestrator-acceptance-* ]] || { echo 'isolated acceptance namespace required' >&2; exit 2; }
mkdir -p "$evidence"
cd "$DEV_REPOSITORY_ROOT"
cargo build --locked -p ayni-orchestrator-control-plane --bin workflow-console --bin workflow-acceptance > "$evidence/build.log" 2>&1
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
console_pid=
forward_pid=
cleanup() {
  if [[ -n "$console_pid" ]]; then kill "$console_pid" 2>/dev/null || true; wait "$console_pid" 2>/dev/null || true; fi
  if [[ -n "$forward_pid" ]]; then kill "$forward_pid" 2>/dev/null || true; wait "$forward_pid" 2>/dev/null || true; fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
kubectl --context "$context" --namespace "$namespace" port-forward --address 127.0.0.1 \
  "pod/$pod" "$upstream:50051" > "$evidence/port-forward.log" 2>&1 &
forward_pid=$!
target/debug/workflow-console "127.0.0.1:$port" "127.0.0.1:$upstream" > "$evidence/console.log" 2>&1 &
console_pid=$!
python3 - "$port" "$console_pid" "$upstream" <<'PY'
import os,socket,sys,time,urllib.request
for attempt in range(100):
    os.kill(int(sys.argv[2]),0)
    try:
        with socket.create_connection(('127.0.0.1',int(sys.argv[3])),timeout=1):pass
        urllib.request.urlopen(f'http://127.0.0.1:{sys.argv[1]}/',timeout=1).close()
        break
    except OSError:
        if attempt==99: raise
        time.sleep(.1)
PY
if [[ "${3:-}" == --fresh-continuation ]]; then
  selection="$evidence/selection.json"
  runtime="${AYNI_ORCHESTRATOR_QUALITY_RUNTIME_IMAGE:-$(python3 -c 'import json; print(json.load(open(".ayni/task-quality-runtime.json"))["runtime_image"])')}"
  ORCHESTRATOR_TASK_QUALITY_RUNTIME_IMAGE="$runtime" target/debug/workflow-acceptance \
    --endpoint "http://127.0.0.1:$upstream" --prepare-task-recovery-review > "$selection"
fi
profile=task-live-acceptance.mjs
[[ "${3:-}" != --recovery && "${3:-}" != --continuation && "${3:-}" != --fresh-continuation ]] || profile=recovery-acceptance.mjs
export TASK_RECOVERY_CONFIRM=false
[[ "${3:-}" != --continuation && "${3:-}" != --fresh-continuation ]] || export TASK_RECOVERY_CONFIRM=true
CONSOLE_URL="http://127.0.0.1:$port" TASK_CONSOLE_SELECTION_FILE="$selection" \
  TASK_CONSOLE_EVIDENCE_DIRECTORY="$evidence" node "apps/console/$profile"
