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

kubectl --context "$context" --namespace "$namespace" get service/control-plane >/dev/null
# The first optional deployment takes ownership from the monitoring resources
# that older core overlays managed. No product resource is present in this render.
"$repository_root/scripts/local/render-observability.sh" \
  | kubectl --context "$context" apply --server-side --force-conflicts \
      --field-manager ayni-orchestrator-observability -f -

# Stable ConfigMap names make teardown predictable. Restart disposable workloads
# so mounted Prometheus and Grafana configuration always matches this checkout.
kubectl --context "$context" --namespace "$namespace" rollout restart \
  deployment/prometheus deployment/kube-state-metrics deployment/grafana
for deployment in prometheus kube-state-metrics grafana; do
  kubectl --context "$context" --namespace "$namespace" rollout status \
    "deployment/$deployment" --timeout=120s
done
