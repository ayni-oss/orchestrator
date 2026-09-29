#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
if [[ $# -ne 0 ]]; then
  echo "usage: $0" >&2
  exit 2
fi
context="$($repository_root/dev/scripts/require-docker-desktop-context.sh)"
readonly context
namespace="$("$repository_root/dev/scripts/resolve-namespace.sh")"
readonly namespace
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
