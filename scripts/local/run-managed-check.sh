#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
context="$("$repository_root/scripts/local/require-docker-desktop-context.sh")"
readonly context
readonly namespace="ayni-orchestrator-dev"
readonly job_name="managed-check"
readonly placeholder="local/ayni-orchestrator-worker:replace-me"

if kubectl --context "$context" --namespace "$namespace" get job "$job_name" >/dev/null 2>&1; then
  echo "managed-check already exists; inspect or explicitly remove that owned Job before starting another attempt" >&2
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
kubectl kustomize "$repository_root/deploy/managed-check/overlays/docker-desktop" \
  | sed "s|$placeholder|$image_ref|g" \
  | kubectl --context "$context" apply --server-side --field-manager ayni-orchestrator-local -f -

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
