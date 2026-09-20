#!/usr/bin/env bash
set -euo pipefail

# Exercises M1's final-artifact gate. The test-only wrapper removes the
# execution record after a real managed check and before the worker validates
# and retains artifacts. A failed Job with the expected error is a pass.

readonly repository_root="$(git rev-parse --show-toplevel)"
context="$("$repository_root/scripts/local/require-docker-desktop-context.sh")"
readonly context
readonly namespace="ayni-orchestrator-dev"
readonly job_name="managed-check-artifact-rejection"
readonly placeholder="local/ayni-orchestrator-worker:replace-me"

if kubectl --context "$context" --namespace "$namespace" get job "$job_name" >/dev/null 2>&1; then
  echo "$job_name already exists; inspect or explicitly remove that owned Job before starting another attempt" >&2
  exit 1
fi

if ! kubectl --context "$context" --namespace "$namespace" get pvc managed-check-evidence >/dev/null; then
  echo "managed-check-evidence must already exist; create it through the normal managed-check or control-plane deployment first" >&2
  exit 1
fi

image_ref="$("$repository_root/scripts/local/build-worker-image.sh")"
readonly image_ref
"$repository_root/scripts/local/load-image.sh" "$image_ref"
kubectl --context "$context" apply -f - <<EOF
apiVersion: v1
kind: Namespace
metadata:
  name: $namespace
EOF
kubectl kustomize "$repository_root/deploy/managed-check/artifact-rejection" \
  | sed "s|$placeholder|$image_ref|g" \
  | kubectl --context "$context" apply --server-side --field-manager ayni-orchestrator-local -f -

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
