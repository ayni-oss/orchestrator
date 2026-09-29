#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
readonly chart="$repository_root/dev/charts/ayni-orchestrator-local"
readonly scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

if [[ $# -ne 0 ]]; then
  echo "usage: $0" >&2
  exit 2
fi
if ! command -v helm >/dev/null 2>&1; then
  echo "Helm is required to build local chart dependencies" >&2
  exit 1
fi

# Use isolated repository state rather than changing the operator's Helm
# configuration. Chart.lock remains the source of dependency coordinates and
# versions; container images are pinned separately by digest in values.yaml.
export HELM_REPOSITORY_CONFIG="$scratch/repositories.yaml"
export HELM_REPOSITORY_CACHE="$scratch/cache"
export HELM_PLUGINS="$scratch/plugins"
mkdir -p "$scratch/cache" "$scratch/plugins"
helm repo add prometheus-community https://prometheus-community.github.io/helm-charts >/dev/null
helm repo add grafana https://grafana.github.io/helm-charts >/dev/null
helm dependency build "$chart" >/dev/null
