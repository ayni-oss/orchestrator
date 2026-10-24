#!/usr/bin/env bash
set -euo pipefail
readonly dev_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$dev_directory/lib/common.sh"
export HELM_NO_PLUGINS=1
readonly context="$(dev_context)" namespace="$(dev_namespace)"
[[ $# -eq 1 && "$1" == /* && ! -e "$1" && "$namespace" == ayni-orchestrator-acceptance-* ]] || {
  echo 'usage: assessment-acceptance.sh ABSOLUTE_NEW_EVIDENCE_DIRECTORY (owned acceptance namespace required)' >&2; exit 2;
}
readonly evidence="$1"
mkdir -p "$evidence"
console_pid= grpc_pid= fixture_pid=
cleanup() {
  for pid in "$console_pid" "$grpc_pid" "$fixture_pid"; do
    [[ -z "$pid" ]] || kill "$pid" 2>/dev/null || true
  done
}
trap cleanup EXIT INT TERM
cd "$DEV_REPOSITORY_ROOT"
cargo build --locked -p ayni-orchestrator-control-plane --bin workflow-console --bin workflow-prepare-task > "$evidence/helper-build.txt" 2>&1
# This ConfigMap contains only the committed deterministic server, never a real
# provider credential. The profile provisions its own public fixture credential.
kubectl --context "$context" --namespace "$namespace" create configmap provider-fixture \
  --from-file=server.mjs=dev/fixtures/provider/server.mjs --dry-run=client -o yaml \
  | kubectl --context "$context" --namespace "$namespace" apply -f - > "$evidence/provider-config.txt"
kubectl --context "$context" --namespace "$namespace" apply -f dev/fixtures/provider/workload.yaml > "$evidence/provider-deploy.txt"
kubectl --context "$context" --namespace "$namespace" rollout restart deployment/provider-fixture >> "$evidence/provider-deploy.txt"
kubectl --context "$context" --namespace "$namespace" rollout status deployment/provider-fixture --timeout=2m >> "$evidence/provider-deploy.txt"
helm upgrade --install "$DEV_RELEASE_NAME" "$DEV_CHART" --reset-then-reuse-values \
  --kube-context "$context" --namespace "$namespace" \
  --set ayni-orchestrator.taskExecution.fixtureEnabled=true \
  --set ayni-orchestrator.providers.openai.enabled=true \
  --set ayni-orchestrator.providers.openai.fixtureEgress=true \
  --set-string "ayni-orchestrator.providers.openai.baseUrl=http://provider-fixture.$namespace.svc:8080/v1" \
  --wait --timeout=5m > "$evidence/provider-binding.txt"
read -r grpc_port console_port fixture_port < <(python3 - <<'PY'
import socket
sockets=[socket.socket() for _ in range(3)]
for sock in sockets: sock.bind(('127.0.0.1',0))
print(*(sock.getsockname()[1] for sock in sockets))
for sock in sockets: sock.close()
PY
)
pod="$(kubectl --context "$context" --namespace "$namespace" get pods -l app.kubernetes.io/name=ayni-orchestrator-control-plane --sort-by=.metadata.creationTimestamp -o jsonpath='{.items[-1:].metadata.name}')"
kubectl --context "$context" --namespace "$namespace" port-forward --address 127.0.0.1 "pod/$pod" "$grpc_port:50051" > "$evidence/grpc-forward.txt" 2>&1 &
grpc_pid=$!
kubectl --context "$context" --namespace "$namespace" port-forward --address 127.0.0.1 service/provider-fixture "$fixture_port:8080" > "$evidence/fixture-forward.txt" 2>&1 &
fixture_pid=$!
for _ in $(seq 1 100); do
  if grep -q "Forwarding from 127.0.0.1:$grpc_port" "$evidence/grpc-forward.txt" && grep -q "Forwarding from 127.0.0.1:$fixture_port" "$evidence/fixture-forward.txt"; then break; fi
  kill -0 "$grpc_pid" && kill -0 "$fixture_pid" || { echo 'assessment forwards exited' >&2; exit 1; }
  sleep .1
done
"$DEV_REPOSITORY_ROOT/target/debug/workflow-console" "127.0.0.1:$console_port" "127.0.0.1:$grpc_port" > "$evidence/console.txt" 2>&1 &
console_pid=$!
CONSOLE_URL="http://127.0.0.1:$console_port" PROVIDER_FIXTURE_URL="http://127.0.0.1:$fixture_port" \
  AYNI_TRUSTED_GRPC_ADDRESS="127.0.0.1:$grpc_port" \
  node apps/console/assessment-acceptance.mjs "$evidence/scenario" > "$evidence/scenario.txt" 2>&1
