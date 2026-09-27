#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
if [[ $# -ne 0 ]]; then
  echo "usage: $0" >&2
  exit 2
fi
context="$($repository_root/scripts/local/require-docker-desktop-context.sh)"
readonly context
readonly namespace="${AYNI_ORCHESTRATOR_NAMESPACE:-ayni-orchestrator-dev}"
readonly local_port="${AYNI_ORCHESTRATOR_LOCAL_PORT:-18080}"
readonly prometheus_port="${AYNI_ORCHESTRATOR_PROMETHEUS_PORT:-19090}"
readonly grafana_port="${AYNI_ORCHESTRATOR_GRAFANA_PORT:-13000}"
readonly response_headers="$(mktemp /tmp/ayni-orchestrator-response-headers.XXXXXX)"

kubectl --context "$context" --namespace "$namespace" port-forward service/control-plane "$local_port:8080" >/tmp/ayni-orchestrator-port-forward.log 2>&1 &
readonly port_forward_pid="$!"
kubectl --context "$context" --namespace "$namespace" port-forward service/prometheus "$prometheus_port:9090" >/tmp/ayni-orchestrator-prometheus-port-forward.log 2>&1 &
readonly prometheus_port_forward_pid="$!"
kubectl --context "$context" --namespace "$namespace" port-forward service/grafana "$grafana_port:3000" >/tmp/ayni-orchestrator-grafana-port-forward.log 2>&1 &
readonly grafana_port_forward_pid="$!"
cleanup() {
  kill "$port_forward_pid" "$prometheus_port_forward_pid" "$grafana_port_forward_pid" 2>/dev/null || true
  wait "$port_forward_pid" "$prometheus_port_forward_pid" "$grafana_port_forward_pid" 2>/dev/null || true
  rm -f "$response_headers"
}
trap cleanup EXIT

check_once() {
  model_catalog="$(curl --fail --silent --show-error \
    "http://127.0.0.1:$local_port/v1/provider-models")" || return 1
  grep -q '"provider":"openai"' <<<"$model_catalog" || return 1
  response="$(curl --fail --silent --show-error --dump-header "$response_headers" \
    -H 'x-request-id: local-readiness-check' \
    "http://127.0.0.1:$local_port/readyz" 2>/dev/null)" || return 1
  grep -qi '^x-request-id: local-readiness-check' "$response_headers" || return 1
  grep -q '"pilot_ready":true' <<<"$response" || return 1
  product_html="$(curl --fail --silent --show-error "http://127.0.0.1:$local_port/")" || return 1
  grep -q 'Ayni Orchestrator' <<<"$product_html" || return 1
  curl --fail --silent --show-error "http://127.0.0.1:$local_port/v1/repository-bindings" >/dev/null || return 1
  curl --fail --silent --show-error "http://127.0.0.1:$local_port/v1/development-tasks" >/dev/null || return 1
  metrics="$(curl --fail --silent --show-error "http://127.0.0.1:$local_port/metrics")" || return 1
  grep -q 'orchestrator_model_interactions_total' <<<"$metrics" || return 1
  curl --fail --silent --show-error "http://127.0.0.1:$prometheus_port/-/ready" >/dev/null || return 1
  prometheus_query="$(curl --get --fail --silent --show-error \
    --data-urlencode 'query=up{job="ayni-orchestrator-control-plane"} == 1' \
    "http://127.0.0.1:$prometheus_port/api/v1/query")" || return 1
  grep -Fq '"result":[{' <<<"$prometheus_query" || return 1
  grafana_health="$(curl --fail --silent --show-error \
    "http://127.0.0.1:$grafana_port/api/health")" || return 1
  grep -Eq '"database"[[:space:]]*:[[:space:]]*"ok"' <<<"$grafana_health" || return 1
}

for _ in {1..20}; do
  if check_once; then
    printf '%s\n' "$response"
    exit 0
  fi
  sleep 1
done

cat /tmp/ayni-orchestrator-port-forward.log >&2
cat /tmp/ayni-orchestrator-prometheus-port-forward.log >&2
cat /tmp/ayni-orchestrator-grafana-port-forward.log >&2
exit 1
