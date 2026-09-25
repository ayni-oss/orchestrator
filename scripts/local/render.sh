#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
if [[ $# -ne 2 ]]; then
  echo "usage: scripts/local/render.sh CONTROL_PLANE_IMAGE_REF WORKER_IMAGE_REF" >&2
  exit 2
fi
readonly control_plane_image_ref="$1"
readonly worker_image_ref="$2"
readonly default_namespace="ayni-orchestrator-dev"
readonly namespace="${AYNI_ORCHESTRATOR_NAMESPACE:-$default_namespace}"
readonly control_plane_placeholder="local/ayni-orchestrator-control-plane:replace-me"
readonly worker_placeholder="local/ayni-orchestrator-worker:replace-me"

if [[ ! "$namespace" =~ ^[a-z0-9]([-a-z0-9]*[a-z0-9])?$ || ${#namespace} -gt 63 ]]; then
  echo "namespace must be a valid DNS label" >&2
  exit 1
fi

for image_ref in "$control_plane_image_ref" "$worker_image_ref"; do
  if [[ -z "$image_ref" || "$image_ref" =~ [[:space:]] || "$image_ref" == *'&'* || "$image_ref" == *'|'* ]]; then
    echo "image reference is empty or contains an unsupported character" >&2
    exit 1
  fi
done

kubectl kustomize "$repository_root/deploy/overlays/docker-desktop" \
  | sed -e "s|$control_plane_placeholder|$control_plane_image_ref|g" \
        -e "s|$worker_placeholder|$worker_image_ref|g" \
        -e "s|$default_namespace|$namespace|g"
