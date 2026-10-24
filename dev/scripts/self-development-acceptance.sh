#!/usr/bin/env bash
set -euo pipefail
readonly dev_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$dev_directory/lib/common.sh"
source "$dev_directory/lib/images.sh"
readonly context="$(dev_context)" namespace="$(dev_namespace)"
[[ $# -eq 2 && "$1" == /* && ! -e "$1" && "$2" =~ @sha256:[a-f0-9]{64}$ && "$namespace" == ayni-orchestrator-acceptance-* ]] || {
  echo 'usage: dev/scripts/self-development-acceptance.sh ABSOLUTE_NEW_EVIDENCE_DIRECTORY DIGEST_PINNED_ORCHESTRATOR_RUNTIME_IMAGE' >&2; exit 2;
}
readonly evidence="$1" runtime="$2"
mkdir -p "$evidence"
console_pid= grpc_pid= fixture_pid=
status=failed
cleanup() {
  for pid in "$console_pid" "$grpc_pid"; do [[ -z "$pid" ]] || kill "$pid" 2>/dev/null || true; done
  printf '{"result":"%s","profile":"orchestrator-self-development","coding":"deterministic-explicit-fixture","live_github":false,"cleanup":"retained_authoritative_stores"}\n' "$status" > "$evidence/outcome.json"
}
trap cleanup EXIT INT TERM
cd "$DEV_REPOSITORY_ROOT"
# Only committed source enters the snapshot. The disposable checkout's fixture
# remote identity is deliberate and recorded separately from its actual lineage.
git diff --quiet HEAD -- backend Cargo.toml Cargo.lock proto .ayni.toml .ayni.lock || {
  echo 'commit the frozen Rust/protocol/managed inputs before self-development acceptance' >&2; exit 1;
}
cargo build --locked -p ayni-orchestrator-control-plane \
  --bin workflow-console --bin workflow-prepare-task --bin workflow-publish-task > "$evidence/host-helper-build.txt" 2>&1
fixture_base="$(dev_build_image publication-fixture)"
printf '%s\n' "$fixture_base" > "$evidence/fixture-base-image.txt"
read -r grpc_port console_port < <(python3 - <<'PY'
import socket
sockets=[socket.socket() for _ in range(2)]
for sock in sockets: sock.bind(('127.0.0.1',0))
print(*(sock.getsockname()[1] for sock in sockets))
for sock in sockets: sock.close()
PY
)
pod="$(kubectl --context "$context" --namespace "$namespace" get pods -l app.kubernetes.io/name=ayni-orchestrator-control-plane --sort-by=.metadata.creationTimestamp -o jsonpath='{.items[-1:].metadata.name}')"
kubectl --context "$context" --namespace "$namespace" port-forward --address 127.0.0.1 \
  "pod/$pod" "$grpc_port:50051" > "$evidence/grpc-forward.txt" 2>&1 &
grpc_pid=$!
for _ in $(seq 1 100); do
  grep -q "Forwarding from 127.0.0.1:$grpc_port" "$evidence/grpc-forward.txt" && break
  kill -0 "$grpc_pid" 2>/dev/null || { cat "$evidence/grpc-forward.txt" >&2; exit 1; }
  sleep .1
done
"$DEV_REPOSITORY_ROOT/target/debug/workflow-console" "127.0.0.1:$console_port" "127.0.0.1:$grpc_port" > "$evidence/console.txt" 2>&1 &
console_pid=$!
node "$dev_directory/lib/self-development.mjs" "$DEV_REPOSITORY_ROOT" "$evidence" "$namespace" "$runtime" "$grpc_port" "$console_port" "$fixture_base"
status=passed
printf 'Own-source managed self-development fixture passed; evidence=%s\n' "$evidence"
