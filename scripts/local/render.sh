#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
readonly control_plane_image_ref="${1:?usage: scripts/local/render.sh CONTROL_PLANE_IMAGE_REF WORKER_IMAGE_REF [MODE [NAMESPACE]]}"
readonly worker_image_ref="${2:?usage: scripts/local/render.sh CONTROL_PLANE_IMAGE_REF WORKER_IMAGE_REF [MODE [NAMESPACE]]}"
readonly mode="${3:-m2}"
readonly default_namespace="ayni-orchestrator-dev"
readonly namespace="${4:-${AYNI_ORCHESTRATOR_NAMESPACE:-$default_namespace}}"
readonly control_plane_placeholder="local/ayni-orchestrator-control-plane:replace-me"
readonly worker_placeholder="local/ayni-orchestrator-worker:replace-me"

if [[ ! "$namespace" =~ ^[a-z0-9]([-a-z0-9]*[a-z0-9])?$ || ${#namespace} -gt 63 ]]; then
  echo "namespace must be a valid DNS label" >&2
  exit 1
fi

for image_ref in "$control_plane_image_ref" "$worker_image_ref"; do
  if [[ "$image_ref" == *$'\n'* || "$image_ref" == *'&'* || "$image_ref" == *'|'* ]]; then
    echo "image reference contains an unsupported character" >&2
    exit 1
  fi
done

case "$mode" in
  m2) overlay="docker-desktop" ;;
  m3) overlay="docker-desktop-m3" ;;
  m4) overlay="docker-desktop-m4" ;;
  *) echo "unsupported deployment mode: $mode" >&2; exit 1 ;;
esac

kubectl kustomize "$repository_root/deploy/overlays/$overlay" \
  | sed -e "s|$control_plane_placeholder|$control_plane_image_ref|g" \
        -e "s|$worker_placeholder|$worker_image_ref|g" \
        -e "s|$default_namespace|$namespace|g"
