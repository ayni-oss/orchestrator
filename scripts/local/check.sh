#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
context="$($repository_root/scripts/local/require-docker-desktop-context.sh)"
readonly context
readonly namespace="ayni-orchestrator-dev"
readonly local_port="18080"
readonly mode="${1:-m2}"

kubectl --context "$context" --namespace "$namespace" port-forward service/control-plane "$local_port:8080" >/tmp/ayni-orchestrator-port-forward.log 2>&1 &
readonly port_forward_pid="$!"
cleanup() {
  kill "$port_forward_pid" 2>/dev/null || true
  wait "$port_forward_pid" 2>/dev/null || true
}
trap cleanup EXIT

for _ in {1..20}; do
  if response="$(curl --fail --silent --show-error "http://127.0.0.1:$local_port/readyz" 2>/dev/null)"; then
    if [[ "$mode" == "m4" ]]; then
      grep -q '"pilot_ready":true' <<<"$response"
      curl --fail --silent --show-error "http://127.0.0.1:$local_port/" | grep -q 'Ayni Orchestrator'
      curl --fail --silent --show-error "http://127.0.0.1:$local_port/v1/repository-bindings" >/dev/null
      curl --fail --silent --show-error "http://127.0.0.1:$local_port/v1/development-tasks" >/dev/null
    fi
    printf '%s\n' "$response"
    exit 0
  fi
  sleep 1
done

cat /tmp/ayni-orchestrator-port-forward.log >&2
exit 1
