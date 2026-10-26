#!/usr/bin/env bash
set -euo pipefail
readonly dev_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$dev_directory/lib/common.sh"
context="$(dev_context)"
namespace="$(dev_namespace)"
readonly port="${1:-3000}"
if [[ ! "$port" =~ ^[0-9]+$ ]] || (( port < 1024 || port > 65535 )); then
  echo 'usage: dev/ctl gateway [PORT from 1024 to 65535]' >&2
  exit 2
fi
pod="$(kubectl --context "$context" --namespace "$namespace" get pods \
  -l app.kubernetes.io/name=ayni-orchestrator-control-plane --sort-by=.metadata.creationTimestamp \
  -o jsonpath='{.items[-1:].metadata.name}')"
log="$(mktemp)"
forward_pid=
cleanup() {
  if [[ -n "$forward_pid" ]]; then kill "$forward_pid" 2>/dev/null || true; fi
  rm -f "$log"
}
trap cleanup EXIT INT TERM
kubectl --context "$context" --namespace "$namespace" port-forward --address 127.0.0.1 \
  "pod/$pod" :50051 > "$log" 2>&1 &
forward_pid=$!
upstream=
for _ in $(seq 1 100); do
  upstream="$(sed -nE 's/^Forwarding from 127\.0\.0\.1:([0-9]+) -> 50051$/\1/p' "$log" | head -1)"
  [[ -z "$upstream" ]] || break
  if ! kill -0 "$forward_pid" 2>/dev/null; then cat "$log" >&2; exit 1; fi
  sleep .1
done
if [[ -z "$upstream" ]]; then echo 'local gRPC port-forward did not become ready' >&2; exit 1; fi
cd "$DEV_REPOSITORY_ROOT"
cargo run --locked -p ayni-orchestrator-control-plane --bin workflow-gateway -- "127.0.0.1:$port" "127.0.0.1:$upstream"
