#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
if [[ $# -ne 0 ]]; then
  echo "usage: $0" >&2
  exit 2
fi
readonly default_namespace="ayni-orchestrator-dev"
readonly namespace="${AYNI_ORCHESTRATOR_NAMESPACE:-$default_namespace}"

if [[ ! "$namespace" =~ ^[a-z0-9]([-a-z0-9]*[a-z0-9])?$ || ${#namespace} -gt 63 ]]; then
  echo "namespace must be a valid DNS label" >&2
  exit 1
fi

kubectl kustomize "$repository_root/deploy/observability/docker-desktop" \
  | sed -e "s|$default_namespace|$namespace|g"
