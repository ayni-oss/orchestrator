#!/usr/bin/env bash
set -euo pipefail

readonly control_plane_image_ref="${1:?usage: dev/scripts/prune-project-images.sh CONTROL_PLANE_IMAGE_REF WORKER_IMAGE_REF WEB_IMAGE_REF}"
readonly worker_image_ref="${2:?usage: dev/scripts/prune-project-images.sh CONTROL_PLANE_IMAGE_REF WORKER_IMAGE_REF WEB_IMAGE_REF}"
readonly web_image_ref="${3:?usage: dev/scripts/prune-project-images.sh CONTROL_PLANE_IMAGE_REF WORKER_IMAGE_REF WEB_IMAGE_REF}"
readonly node_name="desktop-control-plane"
node_id="$(docker inspect "$node_name" --format '{{.Id}}')"
readonly node_id
temporary_directory="$(mktemp -d)"
readonly temporary_directory
trap 'rm -rf "$temporary_directory"' EXIT
readonly keep="$temporary_directory/keep"

printf '%s\n%s\n%s\n' "$control_plane_image_ref" "$worker_image_ref" "$web_image_ref" > "$keep"
kubectl --context docker-desktop get pods --all-namespaces \
  -o jsonpath='{range .items[*]}{range .spec.initContainers[*]}{.image}{"\n"}{end}{range .spec.containers[*]}{.image}{"\n"}{end}{end}' \
  | grep '^local/ayni-orchestrator-' >> "$keep" || true
sort -u "$keep" -o "$keep"

# Docker Desktop's Kubernetes node has a separate containerd image store.
docker exec "$node_id" ctr --namespace k8s.io images list --quiet \
  | { grep '^docker.io/local/ayni-orchestrator-' || true; } \
  | while IFS= read -r runtime_ref; do
      local_ref="${runtime_ref#docker.io/}"
      if ! grep -Fxq "$local_ref" "$keep"; then
        docker exec "$node_id" ctr --namespace k8s.io images remove "$runtime_ref" >/dev/null
        echo "removed obsolete node image $local_ref" >&2
      fi
    done

# Remove matching obsolete tags from the host image store without touching
# unrelated images, build cache, volumes, PVC data, or credentials.
docker image list --format '{{.Repository}}:{{.Tag}}' \
  | { grep '^local/ayni-orchestrator-' || true; } \
  | while IFS= read -r image_ref; do
      if ! grep -Fxq "$image_ref" "$keep"; then
        docker image remove "$image_ref" >/dev/null
        echo "removed obsolete host image $image_ref" >&2
      fi
    done
