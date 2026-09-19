#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
readonly control_plane_image_ref="${1:?usage: scripts/local/render.sh CONTROL_PLANE_IMAGE_REF WORKER_IMAGE_REF}"
readonly worker_image_ref="${2:?usage: scripts/local/render.sh CONTROL_PLANE_IMAGE_REF WORKER_IMAGE_REF}"
readonly control_plane_placeholder="local/ayni-orchestrator-control-plane:replace-me"
readonly worker_placeholder="local/ayni-orchestrator-worker:replace-me"

for image_ref in "$control_plane_image_ref" "$worker_image_ref"; do
  if [[ "$image_ref" == *$'\n'* || "$image_ref" == *'&'* || "$image_ref" == *'|'* ]]; then
    echo "image reference contains an unsupported character" >&2
    exit 1
  fi
done

kubectl kustomize "$repository_root/deploy/overlays/docker-desktop" \
  | sed -e "s|$control_plane_placeholder|$control_plane_image_ref|g" \
        -e "s|$worker_placeholder|$worker_image_ref|g"
