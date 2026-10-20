#!/usr/bin/env bash
set -euo pipefail

readonly dev_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$dev_directory/lib/common.sh"
source "$dev_directory/lib/images.sh"
export HELM_NO_PLUGINS=1

if [[ $# -ne 0 ]]; then
  echo "usage: dev/ctl deploy" >&2
  exit 2
fi
if ! command -v helm >/dev/null 2>&1; then
  echo "Helm is required to deploy the local chart" >&2
  exit 1
fi
helm_version="$(helm version --template '{{ .Version }}' 2>/dev/null)"
if [[ "${helm_version%%+*}" != v4.1.1 ]]; then
  echo "Helm 4.1.1 is required by the local development workflow; found ${helm_version:-unknown}" >&2
  exit 1
fi

context="$(dev_context)"
namespace="$(dev_namespace)"
"$dev_directory/scripts/configure-development-secrets.sh"
dev_build_chart_dependencies
dev_check_capacity 8
control_plane_image_ref="$(dev_build_image control-plane)"
dev_load_image "$control_plane_image_ref"

echo "renderer=$(helm version --short 2>/dev/null)"
helm upgrade --install "$DEV_RELEASE_NAME" "$DEV_CHART" --reset-values \
  --kube-context "$context" \
  --namespace "$namespace" \
  --set-string "ayni-orchestrator.images.controlPlane=$control_plane_image_ref" \
  --wait \
  --timeout 5m

kubectl --context "$context" --namespace "$namespace" rollout status \
  statefulset/workflow-postgres --timeout=5m
kubectl --context "$context" --namespace "$namespace" rollout status \
  deployment/control-plane --timeout=5m

pod="$(kubectl --context "$context" --namespace "$namespace" get pods \
  -l app.kubernetes.io/name=ayni-orchestrator-control-plane \
  -o jsonpath='{.items[0].metadata.name}')"
actual_image="$(kubectl --context "$context" --namespace "$namespace" get pod "$pod" \
  -o jsonpath='{.spec.containers[?(@.name=="control-plane")].image}')"
actual_image_id="$(kubectl --context "$context" --namespace "$namespace" get pod "$pod" \
  -o jsonpath='{.status.containerStatuses[?(@.name=="control-plane")].imageID}')"
if [[ "$actual_image" != "$control_plane_image_ref" || -z "$actual_image_id" ]]; then
  echo "the ready control-plane pod does not report the deployed image identity" >&2
  exit 1
fi

echo "deployed pod=$pod control_plane_image=$actual_image imageID=$actual_image_id"
dev_prune_images "$control_plane_image_ref"
