#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
context="$("$repository_root/scripts/local/require-docker-desktop-context.sh")"
readonly context
namespace="$("$repository_root/scripts/local/resolve-namespace.sh")"
readonly namespace
readonly release_name="ayni-orchestrator-managed-check"
readonly job_name="managed-check"
readonly chart="$repository_root/deploy/charts/local"

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
"$repository_root/scripts/local/build-chart-dependencies.sh"
if helm_release_exists; then
  echo "$release_name already exists; inspect it, then explicitly uninstall that fixture release before another attempt" >&2
  exit 1
fi
if kubectl --context "$context" --namespace "$namespace" get job "$job_name" >/dev/null 2>&1; then
  echo "$job_name already exists; inspect or explicitly remove that owned Job before starting another attempt" >&2
  exit 1
fi
create_evidence_claim=false
if ! kubectl --context "$context" --namespace "$namespace" get pvc managed-check-evidence >/dev/null 2>&1; then
  create_evidence_claim=true
fi
readonly create_evidence_claim
ownership_args=()
if kubectl --context "$context" --namespace "$namespace" get configmap/managed-check-inputs >/dev/null 2>&1; then
  managed_by="$(kubectl --context "$context" --namespace "$namespace" get configmap/managed-check-inputs \
    -o jsonpath='{.metadata.labels.app\.kubernetes\.io/managed-by}')"
  owner="$(kubectl --context "$context" --namespace "$namespace" get configmap/managed-check-inputs \
    -o jsonpath='{.metadata.annotations.meta\.helm\.sh/release-name}')"
  owner_namespace="$(kubectl --context "$context" --namespace "$namespace" get configmap/managed-check-inputs \
    -o jsonpath='{.metadata.annotations.meta\.helm\.sh/release-namespace}')"
  if [[ "$managed_by" == Helm \
    && ( "$owner" != "$release_name" || "$owner_namespace" != "$namespace" ) ]]; then
    echo "refusing to take managed-check-inputs from Helm release ${owner:-unknown}" >&2
    exit 1
  fi
  ownership_args+=(--take-ownership)
fi
readonly ownership_args

image_ref="$("$repository_root/scripts/local/build-worker-image.sh")"
readonly image_ref
"$repository_root/scripts/local/load-image.sh" "$image_ref"
helm install "$release_name" "$chart" \
  --kube-context "$context" \
  --namespace "$namespace" \
  --create-namespace \
  "${ownership_args[@]}" \
  --set ayni-orchestrator.enabled=false \
  --set observability.enabled=false \
  --set fixtures.managedCheck.enabled=true \
  --set "fixtures.managedCheck.createEvidenceClaim=$create_evidence_claim" \
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
