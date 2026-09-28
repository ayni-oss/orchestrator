#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 0 ]]; then
  echo "usage: $0" >&2
  exit 2
fi
readonly namespace="${AYNI_ORCHESTRATOR_NAMESPACE:-ayni-orchestrator-dev}"
if [[ ! "$namespace" =~ ^[a-z0-9]([-a-z0-9]*[a-z0-9])?$ || ${#namespace} -gt 63 ]]; then
  echo "AYNI_ORCHESTRATOR_NAMESPACE must be a valid DNS label" >&2
  exit 1
fi
printf '%s\n' "$namespace"
