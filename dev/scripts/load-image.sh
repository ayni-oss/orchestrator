#!/usr/bin/env bash
set -euo pipefail

readonly image_ref="${1:?usage: dev/scripts/load-image.sh IMAGE_REF}"
readonly node_name="desktop-control-plane"
readonly runtime_image_ref="docker.io/$image_ref"
node_id="$(docker inspect "$node_name" --format '{{.Id}}')"
node_labels="$(docker inspect "$node_name" --format '{{index .Config.Labels "io.x-k8s.kind.cluster"}} {{index .Config.Labels "io.x-k8s.kind.role"}}')"

if [[ "$node_labels" != "desktop control-plane" ]]; then
  echo "Docker Desktop node $node_name is not the expected kind control-plane node" >&2
  exit 1
fi

docker save "$image_ref" | docker exec -i "$node_id" ctr --namespace k8s.io images import - >&2
if ! docker exec "$node_id" ctr --namespace k8s.io images list --quiet | grep -Fxq "$runtime_image_ref"; then
  echo "Docker Desktop node did not retain image $image_ref" >&2
  exit 1
fi

echo "loaded image=$image_ref node=$node_name" >&2
