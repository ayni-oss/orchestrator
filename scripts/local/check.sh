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
readonly grpc_port="${AYNI_ORCHESTRATOR_GRPC_LOCAL_PORT:-15051}"
readonly response_headers="$(mktemp /tmp/ayni-orchestrator-response-headers.XXXXXX)"
readonly http_forward_log="$(mktemp /tmp/ayni-orchestrator-http-forward.XXXXXX)"
readonly grpc_forward_log="$(mktemp /tmp/ayni-orchestrator-grpc-forward.XXXXXX)"

if ! command -v grpcurl >/dev/null 2>&1; then
  echo "grpcurl is required for the native gRPC smoke check" >&2
  exit 1
fi

kubectl --context "$context" --namespace "$namespace" port-forward --address 127.0.0.1 \
  service/control-plane "$local_port:8080" >"$http_forward_log" 2>&1 &
readonly http_forward_pid="$!"
kubectl --context "$context" --namespace "$namespace" port-forward --address 127.0.0.1 \
  service/control-plane "$grpc_port:50051" >"$grpc_forward_log" 2>&1 &
readonly grpc_forward_pid="$!"
cleanup() {
  kill "$http_forward_pid" "$grpc_forward_pid" 2>/dev/null || true
  wait "$http_forward_pid" "$grpc_forward_pid" 2>/dev/null || true
  rm -f "$response_headers" "$http_forward_log" "$grpc_forward_log"
}
trap cleanup EXIT

check_once() {
  curl --fail --silent --show-error "http://127.0.0.1:$local_port/livez" >/dev/null || return 1
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
  grep -q 'orchestrator_development_tasks' <<<"$metrics" || return 1
  grep -q 'orchestrator_model_tokens_total' <<<"$metrics" || return 1
  grpc_services="$(grpcurl -plaintext -max-time 5 "127.0.0.1:$grpc_port" list)" || return 1
  grep -q '^ayni.orchestrator.v1.RepositoryService$' <<<"$grpc_services" || return 1
  grep -q '^ayni.orchestrator.v1.ProviderService$' <<<"$grpc_services" || return 1
  grep -q '^ayni.orchestrator.v1.DevelopmentTaskService$' <<<"$grpc_services" || return 1
  grpc_health="$(grpcurl -plaintext -max-time 5 \
    -d '{"service":"ayni.orchestrator.v1.RepositoryService"}' \
    "127.0.0.1:$grpc_port" grpc.health.v1.Health/Check)" || return 1
  grep -q '"status": "SERVING"' <<<"$grpc_health" || return 1
  grpc_models="$(grpcurl -plaintext -max-time 10 -H 'x-request-id: local-grpc-check' -d '{}' \
    "127.0.0.1:$grpc_port" ayni.orchestrator.v1.ProviderService/ListProviderModels)" || return 1
  grep -q '"provider": "openai"' <<<"$grpc_models" || return 1
}

for _ in {1..20}; do
  if check_once; then
    printf '%s\n' "$response"
    exit 0
  fi
  sleep 1
done

cat "$http_forward_log" "$grpc_forward_log" >&2
exit 1
