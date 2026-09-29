#!/usr/bin/env bash

readonly DEV_DIRECTORY="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly DEV_REPOSITORY_ROOT="$(cd "$DEV_DIRECTORY/.." && pwd)"
readonly DEV_CHART="$DEV_DIRECTORY/charts/ayni-orchestrator-local"
readonly DEV_RELEASE_NAME="ayni-orchestrator"

# Print the only Kubernetes context supported by local tooling.
dev_context() {
  local context
  context="$(kubectl config current-context)"
  if [[ "$context" != docker-desktop ]]; then
    echo "refusing to operate on Kubernetes context '$context'; switch to docker-desktop explicitly" >&2
    return 1
  fi
  printf '%s\n' "$context"
}

# Print the bounded namespace selected for this local environment.
dev_namespace() {
  local namespace="${AYNI_ORCHESTRATOR_NAMESPACE:-ayni-orchestrator-dev}"
  if [[ ! "$namespace" =~ ^[a-z0-9]([-a-z0-9]*[a-z0-9])?$ || ${#namespace} -gt 63 ]]; then
    echo "AYNI_ORCHESTRATOR_NAMESPACE must be a valid Kubernetes namespace" >&2
    return 1
  fi
  printf '%s\n' "$namespace"
}

# Materialize ignored chart archives from the committed lock without changing
# the operator's Helm repository configuration.
dev_build_chart_dependencies() (
  if ! command -v helm >/dev/null 2>&1; then
    echo "Helm is required to build local chart dependencies" >&2
    return 1
  fi
  local scratch
  scratch="$(mktemp -d)"
  trap 'rm -rf "$scratch"' EXIT
  export HELM_REPOSITORY_CONFIG="$scratch/repositories.yaml"
  export HELM_REPOSITORY_CACHE="$scratch/cache"
  export HELM_PLUGINS="$scratch/plugins"
  mkdir -p "$scratch/cache" "$scratch/plugins"
  helm repo add prometheus-community https://prometheus-community.github.io/helm-charts >/dev/null
  helm repo add grafana https://grafana.github.io/helm-charts >/dev/null
  helm dependency build "$DEV_CHART" >/dev/null
)

# Refuse builds that are likely to strand worker Jobs through disk exhaustion.
dev_check_capacity() {
  local minimum_gib="${1:-8}"
  local node_id available_kib required_kib
  node_id="$(docker inspect desktop-control-plane --format '{{.Id}}')"
  available_kib="$(docker exec "$node_id" df -Pk /var | awk 'NR==2 {print $4}')"
  required_kib="$((minimum_gib * 1024 * 1024))"
  if (( available_kib < required_kib )); then
    printf 'Docker Desktop has less than %s GiB free; refusing a build/deploy that could strand managed Jobs\n' "$minimum_gib" >&2
    docker exec "$node_id" df -h /var >&2
    echo 'Expand Docker Desktop storage or explicitly remove unused project images/build cache; never prune PVC volumes implicitly.' >&2
    return 1
  fi
  printf 'Docker Desktop capacity check passed: %s GiB minimum\n' "$minimum_gib" >&2
}
