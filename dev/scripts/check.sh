#!/usr/bin/env bash
set -euo pipefail

readonly dev_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$dev_directory/lib/common.sh"

# Match the checked-out application instead of retaining an obsolete schema pin.
readonly schema_version="$(sed -n 's/^pub(crate) const SCHEMA_VERSION: i64 = \([0-9]*\);$/\1/p' \
  "$DEV_REPOSITORY_ROOT/backend/servers/control-plane/src/adapters/sqlite/mod.rs")"
if [[ -z "$schema_version" ]]; then
  echo "cannot determine the checked-out control-plane schema version" >&2
  exit 1
fi

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
  readonly curl_options=(--connect-timeout 2 --max-time 15 --fail --silent --show-error)

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

  observability_stage="monitoring readiness"
  query_has_result() {
    local query="$1"
    observability_stage="Prometheus query: $query"
    local response
    response="$(curl --get "${curl_options[@]}" \
      --data-urlencode "query=$query" \
      "http://127.0.0.1:$prometheus_port/api/v1/query")" || return 1
    grep -Fq '"result":[{' <<<"$response"
  }

  check_once() {
    observability_stage="Prometheus readiness"
    curl "${curl_options[@]}" \
      "http://127.0.0.1:$prometheus_port/-/ready" >/dev/null || return 1
    for job in \
      ayni-orchestrator-control-plane \
      ayni-orchestrator-otel-collector \
      ayni-orchestrator-jaeger \
      ayni-orchestrator-jetstream \
      ayni-orchestrator-cadvisor \
      ayni-orchestrator-node-storage \
      ayni-orchestrator-kubelet-volumes; do
      query_has_result "up{job=\"$job\"} == 1" || return 1
    done
    query_has_result "orchestrator_clean_v1_schema_info{version=\"$schema_version\"} == 1" || return 1
    query_has_result "container_cpu_usage_seconds_total{namespace=\"$namespace\"}" || return 1
    query_has_result "kubelet_volume_stats_used_bytes{namespace=\"$namespace\"} or container_fs_usage_bytes{job=\"ayni-orchestrator-node-storage\",id=\"/\"}" || return 1
    observability_stage="Grafana health"
    grafana_health="$(curl "${curl_options[@]}" \
      "http://127.0.0.1:$grafana_port/api/health")" || return 1
    grep -Eq '"database"[[:space:]]*:[[:space:]]*"ok"' <<<"$grafana_health" || return 1
    observability_stage="Grafana provisioned dashboard"
    dashboard="$(curl "${curl_options[@]}" \
      "http://127.0.0.1:$grafana_port/api/dashboards/uid/ayni-orchestrator-tasks-resources")" || return 1
    grep -Fq '"uid":"ayni-orchestrator-tasks-resources"' <<<"$dashboard" || return 1
    observability_stage="Grafana execution traces dashboard"
    dashboard="$(curl "${curl_options[@]}" \
      "http://127.0.0.1:$grafana_port/api/dashboards/uid/ayni-orchestrator-traces")" || return 1
    grep -Fq '"uid":"ayni-orchestrator-traces"' <<<"$dashboard" || return 1
    observability_stage="Jaeger service search"
    curl "${curl_options[@]}" \
      "http://127.0.0.1:$grafana_port/api/datasources/uid/jaeger/resources/services" >/dev/null || return 1
    observability_stage="Grafana JetStream dashboard"
    dashboard="$(curl "${curl_options[@]}" \
      "http://127.0.0.1:$grafana_port/api/dashboards/uid/ayni-orchestrator-jetstream")" || return 1
    grep -Fq '"uid":"ayni-orchestrator-jetstream"' <<<"$dashboard" || return 1
  }

  for _ in {1..30}; do
    if check_once; then
      echo "observability targets and dashboard are ready"
      exit 0
    fi
    sleep 2
  done

  printf 'observability smoke check failed at: %s\n' "$observability_stage" >&2
  cat "$prometheus_log" >&2
  cat "$grafana_log" >&2
  exit 1
)

readonly repository_root="$DEV_REPOSITORY_ROOT"
if [[ $# -eq 1 && "$1" == --observability-only ]]; then
  dev_check_observability
  exit
fi
expect_empty=false
if [[ $# -eq 1 && "$1" == --expect-empty ]]; then
  expect_empty=true
elif [[ $# -ne 0 ]]; then
  echo "usage: dev/ctl check [--expect-empty]" >&2
  exit 2
fi
readonly expect_empty
context="$(dev_context)"
namespace="$(dev_namespace)"
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

check_stage="service readiness"
check_once() {
  check_stage="service readiness and schema"
  curl_local "http://127.0.0.1:$operations_port/livez" >/dev/null || return 1
  readiness="$(curl_local --dump-header "$response_headers" \
    -H 'x-request-id: local-readiness-check' \
    "http://127.0.0.1:$operations_port/readyz" 2>/dev/null)" || return 1
  grep -qi '^x-request-id: local-readiness-check' "$response_headers" || return 1
  grep -q '"clean_v1_contract":true' <<<"$readiness" || return 1
  grep -q "\"clean_schema_version\":$schema_version[,}]" <<<"$readiness" || return 1
  metrics="$(curl_local "http://127.0.0.1:$operations_port/metrics")" || return 1
  grep -q "orchestrator_clean_v1_schema_info{version=\"$schema_version\"} 1" <<<"$metrics" || return 1
  grep -q 'orchestrator_grpc_admissions_total' <<<"$metrics" || return 1
  grep -q 'orchestrator_grpc_completions_total' <<<"$metrics" || return 1

  check_stage="web health and repository page"
  curl_local "http://127.0.0.1:$web_port/healthz" | grep -q '"component":"web"' || return 1
  web_html="$(curl_local --dump-header "$response_headers" \
    -H 'x-request-id: local-web-check' "http://127.0.0.1:$web_port/repositories")" || return 1
  grep -qi '^x-request-id: local-web-check' "$response_headers" || return 1
  grep -Eqi '^content-security-policy: .*script-src[^;]*nonce-' "$response_headers" || return 1
  grep -Eq '<script[^>]+nonce=' <<<"$web_html" || return 1
  grep -q 'data-bff-status="connected"' <<<"$web_html" || return 1
  grep -q 'Repositories' <<<"$web_html" || return 1
  check_stage="operations endpoint isolation"
  unsupported_root_status="$(curl --connect-timeout 2 --max-time 10 --silent --show-error \
    --output /dev/null --write-out '%{http_code}' \
    "http://127.0.0.1:$operations_port/")" || return 1
  unsupported_product_status="$(curl --connect-timeout 2 --max-time 10 --silent --show-error \
    --output /dev/null --write-out '%{http_code}' \
    "http://127.0.0.1:$operations_port/v1/development-tasks")" || return 1
  [[ "$unsupported_root_status" == "404" ]] || return 1
  [[ "$unsupported_product_status" == "404" ]] || return 1

  check_stage="native gRPC services and model catalog"
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

  check_stage="empty application resources"
  if [[ "$expect_empty" == true ]]; then
    local response
    response="$(grpcurl -plaintext -max-time 5 -d '{}' "127.0.0.1:$grpc_port" \
      ayni.orchestrator.v1.RepositoryService/ListRepositoryConnections)" || return 1
    ! grep -q 'repositoryConnections' <<<"$response" || return 1
    response="$(grpcurl -plaintext -max-time 5 -d '{}' "127.0.0.1:$grpc_port" \
      ayni.orchestrator.v1.RuntimeProfileService/ListRuntimeProfiles)" || return 1
    ! grep -q 'runtimeProfiles' <<<"$response" || return 1
    response="$(grpcurl -plaintext -max-time 5 -d '{}' "127.0.0.1:$grpc_port" \
      ayni.orchestrator.v1.DevelopmentTaskService/ListDevelopmentTasks)" || return 1
    ! grep -q 'developmentTasks' <<<"$response" || return 1
    local control_plane_logs
    control_plane_logs="$(kubectl --context "$context" --namespace "$namespace" \
      logs deployment/control-plane --since=10m 2>&1)" || return 1
    if grep -Eqi 'retired pilot schema|durable clean-v1 state is corrupt' \
      <<<"$control_plane_logs"; then
      return 1
    fi
  fi
}

for _ in {1..30}; do
  if check_once; then
    printf '%s\n' "$readiness"
    dev_check_observability
    exit 0
  fi
  sleep 1
done

printf 'local smoke check failed at: %s\n' "$check_stage" >&2
cat "$web_forward_log" "$operations_forward_log" "$grpc_forward_log" >&2
exit 1
