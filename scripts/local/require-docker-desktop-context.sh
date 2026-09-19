#!/usr/bin/env bash
set -euo pipefail

readonly context="docker-desktop"

if ! kubectl config get-contexts -o name | grep -Fxq "$context"; then
  echo "required Kubernetes context '$context' is not configured" >&2
  exit 1
fi

if ! kubectl --context "$context" get nodes >/dev/null; then
  echo "Kubernetes context '$context' is unavailable or has no accessible nodes" >&2
  exit 1
fi

echo "$context"
