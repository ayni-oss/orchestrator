#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
if ! command -v helm >/dev/null 2>&1; then
  echo "Helm is required to uninstall the local release" >&2
  exit 1
fi
context="$($repository_root/dev/scripts/require-docker-desktop-context.sh)"
readonly context
namespace="$("$repository_root/dev/scripts/resolve-namespace.sh")"
readonly namespace
readonly release_name="ayni-orchestrator"

# The chart marks all four state claims with helm.sh/resource-policy: keep.
# Uninstall therefore removes the stateless application and observability stack
# while retaining claims, credentials, task Jobs, and the namespace.
helm uninstall "$release_name" \
  --kube-context "$context" \
  --namespace "$namespace" \
  --ignore-not-found
