#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
context="$($repository_root/scripts/local/require-docker-desktop-context.sh)"
readonly context
readonly namespace="ayni-orchestrator-dev"

# Remove only stateless service resources. Task state, checkpoints, managed
# evidence, prepared environments, credentials, Jobs, and the namespace are
# deliberately retained. Cancel active tasks through the UI/API before cleanup.
kubectl --context "$context" --namespace "$namespace" delete \
  deployment/control-plane service/control-plane configmap/control-plane-config \
  role/control-plane-attempt-jobs rolebinding/control-plane-attempt-jobs \
  role/control-plane-development-clone-secrets \
  rolebinding/control-plane-development-clone-secrets \
  serviceaccount/control-plane --ignore-not-found
