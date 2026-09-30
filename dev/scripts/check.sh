#!/usr/bin/env bash
set -euo pipefail

readonly dev_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$dev_directory/lib/common.sh"

# Validate local monitoring RBAC, scrape targets, and the provisioned dashboard.
dev_check_observability() (
  set -euo pipefail
  local context namespace
  context="$(dev_context)"
  namespace="$(dev_namespace)"
  readonly prometheus_port="${AYNI_ORCHESTRATOR_PROMETHEUS_PORT:-19090}"
  readonly grafana_port="${AYNI_ORCHESTRATOR_GRAFANA_PORT:-13000}"
  readonly prometheus_log="$(mktemp /tmp/ayni-orchestrator-prometheus-port-forward.XXXXXX)"
  readonly grafana_log="$(mktemp /tmp/ayni-orchestrator-grafana-port-forward.XXXXXX)"
  readonly curl_options=(--connect-timeout 2 --max-time 5 --fail --silent --show-error)

  kubectl --context "$context" --namespace "$namespace" port-forward \
    service/prometheus "$prometheus_port:9090" \
    >"$prometheus_log" 2>&1 &
  readonly prometheus_port_forward_pid="$!"
  kubectl --context "$context" --namespace "$namespace" port-forward \
    service/grafana "$grafana_port:3000" \
    >"$grafana_log" 2>&1 &
  readonly grafana_port_forward_pid="$!"
  cleanup() {
    kill "$prometheus_port_forward_pid" "$grafana_port_forward_pid" 2>/dev/null || true
    wait "$prometheus_port_forward_pid" "$grafana_port_forward_pid" 2>/dev/null || true
    rm -f "$prometheus_log" "$grafana_log"
  }
  trap cleanup EXIT

  [[ "$(kubectl --context "$context" auth can-i get nodes --subresource=metrics \
    --as="system:serviceaccount:$namespace:prometheus" 2>/dev/null || true)" == "yes" ]]
  [[ "$(kubectl --context "$context" auth can-i get nodes --subresource=proxy \
    --as="system:serviceaccount:$namespace:prometheus" 2>/dev/null || true)" == "no" ]]
  [[ "$(kubectl --context "$context" auth can-i get secrets --namespace "$namespace" \
    --as="system:serviceaccount:$namespace:prometheus" 2>/dev/null || true)" == "no" ]]

  query_has_result() {
    local query="$1"
    local response
    response="$(curl --get "${curl_options[@]}" \
      --data-urlencode "query=$query" \
      "http://127.0.0.1:$prometheus_port/api/v1/query")" || return 1
    grep -Fq '"result":[{' <<<"$response"
  }

  check_once() {
    curl "${curl_options[@]}" \
      "http://127.0.0.1:$prometheus_port/-/ready" >/dev/null || return 1
    for job in \
      ayni-orchestrator-control-plane \
      ayni-orchestrator-kube-state-metrics \
      ayni-orchestrator-cadvisor \
      ayni-orchestrator-node-storage \
      ayni-orchestrator-kubelet-volumes; do
      query_has_result "up{job=\"$job\"} == 1" || return 1
    done
    query_has_result "orchestrator_development_tasks" || return 1
    query_has_result "kube_pod_info{namespace=\"$namespace\"}" || return 1
    query_has_result "container_cpu_usage_seconds_total{namespace=\"$namespace\"}" || return 1
    query_has_result "kubelet_volume_stats_used_bytes{namespace=\"$namespace\"} or container_fs_usage_bytes{job=\"ayni-orchestrator-node-storage\",id=\"/\"}" || return 1
    grafana_health="$(curl "${curl_options[@]}" \
      "http://127.0.0.1:$grafana_port/api/health")" || return 1
    grep -Eq '"database"[[:space:]]*:[[:space:]]*"ok"' <<<"$grafana_health" || return 1
    dashboard="$(curl "${curl_options[@]}" \
      "http://127.0.0.1:$grafana_port/api/dashboards/uid/ayni-orchestrator-tasks-resources")" || return 1
    grep -Fq '"uid":"ayni-orchestrator-tasks-resources"' <<<"$dashboard" || return 1
  }

  for _ in {1..30}; do
    if check_once; then
      echo "observability targets and dashboard are ready"
      exit 0
    fi
    sleep 2
  done

  cat "$prometheus_log" >&2
  cat "$grafana_log" >&2
  exit 1
)

readonly repository_root="$DEV_REPOSITORY_ROOT"
if [[ $# -eq 1 && "$1" == --observability-only ]]; then
  dev_check_observability
  exit
fi
if [[ $# -ne 0 ]]; then
  echo "usage: dev/ctl check" >&2
  exit 2
fi
context="$(dev_context)"
readonly context
namespace="$(dev_namespace)"
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
  grep -q '"clean_v1_contract":true' <<<"$readiness" || return 1
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
    dev_check_observability
    exit 0
  fi
  sleep 1
done

cat "$web_forward_log" "$operations_forward_log" "$grpc_forward_log" >&2
exit 1
