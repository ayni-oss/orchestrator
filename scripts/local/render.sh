#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
if [[ $# -ne 3 ]]; then
  echo "usage: scripts/local/render.sh CONTROL_PLANE_IMAGE_REF WORKER_IMAGE_REF WEB_IMAGE_REF" >&2
  exit 2
fi
readonly control_plane_image_ref="$1"
readonly worker_image_ref="$2"
readonly web_image_ref="$3"
namespace="$("$repository_root/scripts/local/resolve-namespace.sh")"
readonly namespace
readonly deployment_profile="${AYNI_ORCHESTRATOR_DEPLOYMENT_PROFILE:-local}"
readonly chart="$repository_root/deploy/charts/local"

if ! command -v helm >/dev/null 2>&1; then
  echo "Helm is required to render the local chart" >&2
  exit 1
fi
for image_ref in "$control_plane_image_ref" "$worker_image_ref" "$web_image_ref"; do
  if [[ -z "$image_ref" || "$image_ref" =~ [[:space:]] || "$image_ref" == *','* ]]; then
    echo "image reference is empty or contains an unsupported character" >&2
    exit 1
  fi
done
case "$deployment_profile" in
  local) readonly custom_clients=false ;;
  custom-clients) readonly custom_clients=true ;;
  *)
    echo "AYNI_ORCHESTRATOR_DEPLOYMENT_PROFILE must be local or custom-clients" >&2
    exit 1
    ;;
esac

helm template ayni-orchestrator "$chart" \
  --namespace "$namespace" \
  --set-string "images.controlPlane=$control_plane_image_ref" \
  --set-string "images.worker=$worker_image_ref" \
  --set-string "images.web=$web_image_ref" \
  --set "customClients.enabled=$custom_clients"
