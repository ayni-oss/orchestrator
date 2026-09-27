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
"$repository_root/scripts/local/load-image.sh" "$control_plane_image_ref"
"$repository_root/scripts/local/load-image.sh" "$worker_image_ref"

echo "renderer=$(kubectl version --client --output=json | tr -d '\n')"
kubectl --context "$context" apply -f - <<EOF
apiVersion: v1
kind: Namespace
metadata:
  name: $namespace
EOF

"$repository_root/scripts/local/render.sh" \
  "$control_plane_image_ref" "$worker_image_ref" \
  | kubectl --context "$context" apply --server-side --field-manager ayni-orchestrator-local -f -
# Monitoring ConfigMaps are stable names; restart the disposable pods so mounted
# configuration and subPath provisioning files always match this checkout.
kubectl --context "$context" --namespace "$namespace" rollout restart \
  deployment/prometheus deployment/grafana
kubectl --context "$context" --namespace "$namespace" rollout status deployment/control-plane --timeout=120s
kubectl --context "$context" --namespace "$namespace" rollout status deployment/prometheus --timeout=120s
kubectl --context "$context" --namespace "$namespace" rollout status deployment/grafana --timeout=120s

pod=""
actual_image=""
actual_image_id=""
for candidate in $(kubectl --context "$context" --namespace "$namespace" get pods -l app.kubernetes.io/name=ayni-orchestrator-control-plane -o jsonpath='{range .items[*]}{.metadata.name}{"\n"}{end}'); do
  candidate_image="$(kubectl --context "$context" --namespace "$namespace" get pod "$candidate" -o jsonpath='{.spec.containers[0].image}')"
  candidate_image_id="$(kubectl --context "$context" --namespace "$namespace" get pod "$candidate" -o jsonpath='{.status.containerStatuses[0].imageID}')"
  if [[ "$candidate_image" == "$control_plane_image_ref" && -n "$candidate_image_id" ]]; then
    pod="$candidate"
    actual_image="$candidate_image"
    actual_image_id="$candidate_image_id"
    break
  fi
done

if [[ -z "$pod" ]]; then
  echo "no active pod reported the deployed image identity for $control_plane_image_ref" >&2
  exit 1
fi

echo "deployed pod=$pod image=$actual_image imageID=$actual_image_id worker_image=$worker_image_ref"
"$repository_root/scripts/local/prune-project-images.sh" \
  "$control_plane_image_ref" "$worker_image_ref"
