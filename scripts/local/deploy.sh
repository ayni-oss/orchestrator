#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
if [[ $# -ne 0 ]]; then
  echo "usage: $0" >&2
  exit 2
fi
context="$($repository_root/scripts/local/require-docker-desktop-context.sh)"
readonly context
readonly namespace="${AYNI_ORCHESTRATOR_NAMESPACE:-ayni-orchestrator-dev}"
"$repository_root/scripts/local/check-capacity.sh" 12
control_plane_image_ref="$($repository_root/scripts/local/build-image.sh)"
readonly control_plane_image_ref
worker_image_ref="$($repository_root/scripts/local/build-worker-image.sh)"
readonly worker_image_ref
web_image_ref="$($repository_root/scripts/local/build-web-image.sh)"
readonly web_image_ref
"$repository_root/scripts/local/load-image.sh" "$control_plane_image_ref"
"$repository_root/scripts/local/load-image.sh" "$worker_image_ref"
"$repository_root/scripts/local/load-image.sh" "$web_image_ref"

echo "renderer=$(kubectl version --client --output=json | tr -d '\n')"
kubectl --context "$context" apply -f - <<EOF
apiVersion: v1
kind: Namespace
metadata:
  name: $namespace
EOF

# Server-side apply cannot change the default RollingUpdate strategy to Recreate
# while the API-retained rollingUpdate fields are still present. Clear them
# atomically before applying the single-writer-safe Pod template.
if kubectl --context "$context" --namespace "$namespace" get deployment/control-plane >/dev/null 2>&1; then
  kubectl --context "$context" --namespace "$namespace" patch deployment/control-plane \
    --type=strategic \
    -p '{"spec":{"strategy":{"$retainKeys":["type"],"type":"Recreate"}}}' >/dev/null
fi

"$repository_root/scripts/local/render.sh" \
  "$control_plane_image_ref" "$worker_image_ref" "$web_image_ref" \
  | kubectl --context "$context" apply --server-side --field-manager ayni-orchestrator-local -f -
kubectl --context "$context" --namespace "$namespace" rollout status deployment/control-plane --timeout=120s

pod=""
actual_control_plane_image=""
actual_control_plane_image_id=""
actual_worker_image=""
actual_web_image=""
actual_web_image_id=""
for candidate in $(kubectl --context "$context" --namespace "$namespace" get pods -l app.kubernetes.io/name=ayni-orchestrator-control-plane -o jsonpath='{range .items[*]}{.metadata.name}{"\n"}{end}'); do
  candidate_control_plane_image="$(kubectl --context "$context" --namespace "$namespace" get pod "$candidate" -o jsonpath='{.spec.containers[?(@.name=="control-plane")].image}')"
  candidate_control_plane_image_id="$(kubectl --context "$context" --namespace "$namespace" get pod "$candidate" -o jsonpath='{.status.containerStatuses[?(@.name=="control-plane")].imageID}')"
  candidate_worker_image="$(kubectl --context "$context" --namespace "$namespace" get pod "$candidate" -o jsonpath='{.spec.containers[?(@.name=="control-plane")].env[?(@.name=="ORCHESTRATOR_WORKER_IMAGE")].value}')"
  candidate_web_image="$(kubectl --context "$context" --namespace "$namespace" get pod "$candidate" -o jsonpath='{.spec.containers[?(@.name=="web")].image}')"
  candidate_web_image_id="$(kubectl --context "$context" --namespace "$namespace" get pod "$candidate" -o jsonpath='{.status.containerStatuses[?(@.name=="web")].imageID}')"
  if [[ "$candidate_control_plane_image" == "$control_plane_image_ref" \
    && "$candidate_worker_image" == "$worker_image_ref" \
    && "$candidate_web_image" == "$web_image_ref" \
    && -n "$candidate_control_plane_image_id" \
    && -n "$candidate_web_image_id" ]]; then
    pod="$candidate"
    actual_control_plane_image="$candidate_control_plane_image"
    actual_control_plane_image_id="$candidate_control_plane_image_id"
    actual_worker_image="$candidate_worker_image"
    actual_web_image="$candidate_web_image"
    actual_web_image_id="$candidate_web_image_id"
    break
  fi
done

if [[ -z "$pod" ]]; then
  echo "no active pod reported the deployed control-plane, worker, and web image identities" >&2
  exit 1
fi

echo "deployed pod=$pod control_plane_image=$actual_control_plane_image control_plane_imageID=$actual_control_plane_image_id worker_image=$actual_worker_image web_image=$actual_web_image web_imageID=$actual_web_image_id"
"$repository_root/scripts/local/prune-project-images.sh" \
  "$control_plane_image_ref" "$worker_image_ref" "$web_image_ref"
