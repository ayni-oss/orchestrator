#!/usr/bin/env bash
set -euo pipefail
readonly dev_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$dev_directory/lib/common.sh"
context="$(dev_context)"
namespace="$(dev_namespace)"
mode="${1:-}"
if [[ ( ( "$mode" != verify && "$mode" != start ) || $# -ne 4 ) && ( ( "$mode" != submit && "$mode" != submit-start ) || $# -ne 2 ) ]]; then
  echo 'usage: dev/ctl prepare-task verify CHECKOUT SELECTION_JSON FROZEN_REQUEST_JSON | submit FROZEN_REQUEST_JSON' >&2
  exit 2
fi
shift
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
cargo run --locked -p ayni-orchestrator-control-plane --bin workflow-prepare-task -- "$mode" "127.0.0.1:$upstream" "$@"
