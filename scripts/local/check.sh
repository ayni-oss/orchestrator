#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
if [[ $# -ne 0 ]]; then
  echo "usage: $0" >&2
  exit 2
fi
context="$($repository_root/scripts/local/require-docker-desktop-context.sh)"
readonly context
namespace="$("$repository_root/scripts/local/resolve-namespace.sh")"
readonly namespace
readonly web_port="${AYNI_ORCHESTRATOR_WEB_LOCAL_PORT:-18080}"
readonly operations_port="${AYNI_ORCHESTRATOR_OPERATIONS_LOCAL_PORT:-18081}"
readonly grpc_port="${AYNI_ORCHESTRATOR_GRPC_LOCAL_PORT:-15051}"
readonly response_headers="$(mktemp /tmp/ayni-orchestrator-response-headers.XXXXXX)"
readonly web_forward_log="$(mktemp /tmp/ayni-orchestrator-web-forward.XXXXXX)"
readonly operations_forward_log="$(mktemp /tmp/ayni-orchestrator-operations-forward.XXXXXX)"
readonly grpc_forward_log="$(mktemp /tmp/ayni-orchestrator-grpc-forward.XXXXXX)"

if ! command -v grpcurl >/dev/null 2>&1; then
  echo "grpcurl is required for the native gRPC smoke check" >&2
  exit 1
fi

kubectl --context "$context" --namespace "$namespace" port-forward --address 127.0.0.1 \
  service/web "$web_port:8080" >"$web_forward_log" 2>&1 &
readonly web_forward_pid="$!"
kubectl --context "$context" --namespace "$namespace" port-forward --address 127.0.0.1 \
  service/control-plane "$operations_port:8080" >"$operations_forward_log" 2>&1 &
readonly operations_forward_pid="$!"
kubectl --context "$context" --namespace "$namespace" port-forward --address 127.0.0.1 \
  service/control-plane "$grpc_port:50051" >"$grpc_forward_log" 2>&1 &
readonly grpc_forward_pid="$!"
cleanup() {
  kill "$web_forward_pid" "$operations_forward_pid" "$grpc_forward_pid" 2>/dev/null || true
  wait "$web_forward_pid" "$operations_forward_pid" "$grpc_forward_pid" 2>/dev/null || true
  rm -f "$response_headers" "$web_forward_log" "$operations_forward_log" "$grpc_forward_log"
}
trap cleanup EXIT

curl_local() {
  curl --connect-timeout 2 --max-time 10 --fail --silent --show-error "$@"
}

check_once() {
  curl_local "http://127.0.0.1:$operations_port/livez" >/dev/null || return 1
  readiness="$(curl_local --dump-header "$response_headers" \
    -H 'x-request-id: local-readiness-check' \
    "http://127.0.0.1:$operations_port/readyz" 2>/dev/null)" || return 1
  grep -qi '^x-request-id: local-readiness-check' "$response_headers" || return 1
  grep -q '"pilot_ready":true' <<<"$readiness" || return 1
  metrics="$(curl_local "http://127.0.0.1:$operations_port/metrics")" || return 1
  grep -q 'orchestrator_model_interactions_total' <<<"$metrics" || return 1
  grep -q 'orchestrator_development_tasks' <<<"$metrics" || return 1
  grep -q 'orchestrator_model_tokens_total' <<<"$metrics" || return 1

  curl_local "http://127.0.0.1:$web_port/healthz" | grep -q '"component":"web"' || return 1
  web_html="$(curl_local --dump-header "$response_headers" \
    -H 'x-request-id: local-web-check' "http://127.0.0.1:$web_port/")" || return 1
  grep -qi '^x-request-id: local-web-check' "$response_headers" || return 1
  grep -Eqi '^content-security-policy: .*script-src[^;]*nonce-' "$response_headers" || return 1
  grep -Eq '<script[^>]+nonce=' <<<"$web_html" || return 1
  grep -q 'data-bff-status="connected"' <<<"$web_html" || return 1
  grep -q 'data-grpc-provider="openai"' <<<"$web_html" || return 1
  grep -q 'Development control plane' <<<"$web_html" || return 1
  unsupported_root_status="$(curl --connect-timeout 2 --max-time 10 --silent --show-error \
    --output /dev/null --write-out '%{http_code}' \
    "http://127.0.0.1:$operations_port/")" || return 1
  unsupported_product_status="$(curl --connect-timeout 2 --max-time 10 --silent --show-error \
    --output /dev/null --write-out '%{http_code}' \
    "http://127.0.0.1:$operations_port/v1/development-tasks")" || return 1
  [[ "$unsupported_root_status" == "404" ]] || return 1
  [[ "$unsupported_product_status" == "404" ]] || return 1

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

for _ in {1..30}; do
  if check_once; then
    printf '%s\n' "$readiness"
    "$repository_root/scripts/local/check-observability.sh"
    exit 0
  fi
  sleep 1
done

cat "$web_forward_log" "$operations_forward_log" "$grpc_forward_log" >&2
exit 1
