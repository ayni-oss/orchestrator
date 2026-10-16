#!/usr/bin/env bash
# Synthetic trace collection in an isolated Docker Desktop namespace.
set -euo pipefail
root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
source "$root/dev/lib/common.sh"
context=$(dev_context)
namespace="ayni-orchestrator-telemetry-test-$(date +%s)-$$"
scratch=$(mktemp -d)
mkdir -p "$scratch/plugins"
export HELM_PLUGINS="$scratch/plugins"
created=false
pids=()
cleanup() {
  for pid in "${pids[@]:-}"; do
    if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
  done
  if [[ "$created" == true ]]; then
    helm uninstall "$DEV_RELEASE_NAME" --kube-context "$context" -n "$namespace" --ignore-not-found
    kubectl --context "$context" delete namespace "$namespace" --wait=false
  fi
  rm -rf "$scratch"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
dev_build_chart_dependencies
kubectl --context "$context" create namespace "$namespace"
created=true
helm upgrade --install "$DEV_RELEASE_NAME" "$DEV_CHART" \
  --kube-context "$context" -n "$namespace" --wait --timeout=5m \
  --set ayni-orchestrator.enabled=false --set jetstream.enabled=false
ports=()
for mapping in 'grafana:3000' 'otel-collector:4318'; do
  service="${mapping%:*}"; target="${mapping#*:}"
  kubectl --context "$context" -n "$namespace" port-forward "service/$service" ":$target" >"$scratch/$service.log" 2>&1 &
  pids+=("$!")
  port=''
  for _ in {1..50}; do
    port=$(sed -n 's/Forwarding from 127.0.0.1:\([0-9]*\) ->.*/\1/p' "$scratch/$service.log")
    [[ -z "$port" ]] || break
    sleep 0.2
  done
  [[ -n "$port" ]] || { cat "$scratch/$service.log" >&2; exit 1; }
  ports+=("$port")
done
python3 -u "$root/dev/tests/inspect-telemetry.py" "${ports[0]}" "${ports[1]}" "$namespace"
