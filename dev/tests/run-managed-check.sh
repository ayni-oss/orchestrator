#!/usr/bin/env bash
set -euo pipefail

readonly dev_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$dev_directory/lib/common.sh"
source "$dev_directory/lib/images.sh"
readonly repository_root="$DEV_REPOSITORY_ROOT"
context="$(dev_context)"
readonly context
namespace="$(dev_namespace)"
readonly namespace
readonly release_name="ayni-orchestrator-managed-check"
readonly job_name="managed-check"
readonly chart="$repository_root/dev/tests/charts/worker-fixtures"

helm_release_exists() {
  local output
  if output="$(helm status "$release_name" --kube-context "$context" --namespace "$namespace" 2>&1)"; then
    return 0
  fi
  if grep -q 'release: not found' <<<"$output"; then
    return 1
  fi
  echo "unable to inspect Helm release $release_name" >&2
  exit 1
}

if ! command -v helm >/dev/null 2>&1; then
  echo "Helm is required to install the local managed-check fixture" >&2
  exit 1
fi
if helm_release_exists; then
  echo "$release_name already exists; inspect it, then explicitly uninstall that fixture release before another attempt" >&2
  exit 1
fi
if kubectl --context "$context" --namespace "$namespace" get job "$job_name" >/dev/null 2>&1; then
  echo "$job_name already exists; inspect or explicitly remove that owned Job before starting another attempt" >&2
  exit 1
fi
if ! kubectl --context "$context" --namespace "$namespace" get pvc managed-check-evidence >/dev/null 2>&1; then
  echo "managed-check-evidence is missing; deploy the local environment before running this test" >&2
  exit 1
fi
if kubectl --context "$context" --namespace "$namespace" get configmap/managed-check-inputs >/dev/null 2>&1; then
  echo "stale managed-check fixture resources exist; remove them or purge the disposable environment" >&2
  exit 1
fi

image_ref="$(dev_build_image worker)"
readonly image_ref
dev_load_image "$image_ref"
helm install "$release_name" "$chart" \
  --kube-context "$context" \
  --namespace "$namespace" \
  --set fixtures.managedCheck.enabled=true \
  --set-string "fixtures.workerImage=$image_ref" >/dev/null

deadline=$((SECONDS + 900))
while (( SECONDS < deadline )); do
  complete="$(kubectl --context "$context" --namespace "$namespace" get "job/$job_name" -o jsonpath='{.status.conditions[?(@.type=="Complete")].status}')"
  failed="$(kubectl --context "$context" --namespace "$namespace" get "job/$job_name" -o jsonpath='{.status.conditions[?(@.type=="Failed")].status}')"
  if [[ "$complete" == "True" ]]; then
    kubectl --context "$context" --namespace "$namespace" logs "job/$job_name" --container worker --tail=200
    exit 0
  fi
  if [[ "$failed" == "True" ]]; then
    kubectl --context "$context" --namespace "$namespace" logs "job/$job_name" --all-containers --tail=200 >&2 || true
    exit 1
  fi
  sleep 2
done
kubectl --context "$context" --namespace "$namespace" logs "job/$job_name" --all-containers --tail=200 >&2 || true
exit 1
