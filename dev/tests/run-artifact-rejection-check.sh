#!/usr/bin/env bash
set -euo pipefail

# Exercises the managed check's final-artifact gate. The test-only wrapper removes the
# execution record after a real managed check and before the worker validates
# and retains artifacts. A failed Job with the expected error is a pass.

readonly repository_root="$(git rev-parse --show-toplevel)"
context="$("$repository_root/dev/scripts/require-docker-desktop-context.sh")"
readonly context
namespace="$("$repository_root/dev/scripts/resolve-namespace.sh")"
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
  echo "managed-check-evidence must already exist; install the local chart or run the managed-check fixture first" >&2
  exit 1
fi
ownership_args=()
for configmap in managed-check-artifact-rejection-inputs managed-check-artifact-rejection-wrapper; do
  if ! kubectl --context "$context" --namespace "$namespace" get "configmap/$configmap" >/dev/null 2>&1; then
    continue
  fi
  managed_by="$(kubectl --context "$context" --namespace "$namespace" get "configmap/$configmap" \
    -o jsonpath='{.metadata.labels.app\.kubernetes\.io/managed-by}')"
  owner="$(kubectl --context "$context" --namespace "$namespace" get "configmap/$configmap" \
    -o jsonpath='{.metadata.annotations.meta\.helm\.sh/release-name}')"
  owner_namespace="$(kubectl --context "$context" --namespace "$namespace" get "configmap/$configmap" \
    -o jsonpath='{.metadata.annotations.meta\.helm\.sh/release-namespace}')"
  if [[ "$managed_by" == Helm \
    && ( "$owner" != "$release_name" || "$owner_namespace" != "$namespace" ) ]]; then
    echo "refusing to take $configmap from Helm release ${owner:-unknown}" >&2
    exit 1
  fi
  ownership_args+=(--take-ownership)
done
readonly ownership_args

image_ref="$("$repository_root/dev/scripts/build-image.sh" worker)"
readonly image_ref
"$repository_root/dev/scripts/load-image.sh" "$image_ref"
helm install "$release_name" "$chart" \
  --kube-context "$context" \
  --namespace "$namespace" \
  "${ownership_args[@]}" \
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
