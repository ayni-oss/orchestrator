#!/usr/bin/env bash
set -euo pipefail

# Exercises the managed check's final-artifact gate. The test-only wrapper removes the
# execution record after a real managed check and before the worker validates
# and retains artifacts. A failed Job with the expected error is a pass.

readonly dev_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$dev_directory/lib/common.sh"
source "$dev_directory/lib/images.sh"
readonly repository_root="$DEV_REPOSITORY_ROOT"
context="$(dev_context)"
readonly context
namespace="$(dev_namespace)"
readonly namespace
readonly release_name="ayni-orchestrator-artifact-rejection"
readonly job_name="managed-check-artifact-rejection"
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
for configmap in managed-check-artifact-rejection-inputs managed-check-artifact-rejection-wrapper; do
  if kubectl --context "$context" --namespace "$namespace" get "configmap/$configmap" >/dev/null 2>&1; then
    echo "stale artifact-rejection fixture resources exist; remove them or purge the disposable environment" >&2
    exit 1
  fi
done

image_ref="$(dev_build_image worker)"
readonly image_ref
dev_load_image "$image_ref"
helm install "$release_name" "$chart" \
  --kube-context "$context" \
  --namespace "$namespace" \
  --set fixtures.artifactRejection.enabled=true \
  --set-string "fixtures.workerImage=$image_ref" >/dev/null

deadline=$((SECONDS + 900))
while (( SECONDS < deadline )); do
  complete="$(kubectl --context "$context" --namespace "$namespace" get "job/$job_name" -o jsonpath='{.status.conditions[?(@.type=="Complete")].status}')"
  failed="$(kubectl --context "$context" --namespace "$namespace" get "job/$job_name" -o jsonpath='{.status.conditions[?(@.type=="Failed")].status}')"
  if [[ "$complete" == "True" ]]; then
    kubectl --context "$context" --namespace "$namespace" logs "job/$job_name" --all-containers --tail=200 >&2 || true
    echo "$job_name unexpectedly completed" >&2
    exit 1
  fi
  if [[ "$failed" == "True" ]]; then
    logs="$(kubectl --context "$context" --namespace "$namespace" logs "job/$job_name" --container worker --tail=200)"
    printf '%s\n' "$logs"
    if [[ "$logs" == *"missing execution artifact"* ]]; then
      exit 0
    fi
    echo "$job_name failed without the expected missing-artifact rejection" >&2
    exit 1
  fi
  sleep 2
done

kubectl --context "$context" --namespace "$namespace" logs "job/$job_name" --all-containers --tail=200 >&2 || true
echo "$job_name did not reach the expected terminal state within 15 minutes" >&2
exit 1
