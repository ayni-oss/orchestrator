#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
readonly revision="$(git -C "$repository_root" rev-parse --short=12 HEAD)"
readonly requested_image_ref="${1:-}"
readonly build_image_ref="${requested_image_ref:-local/ayni-orchestrator-control-plane:build-git-${revision}-$$}"

if [[ -n "$(git -C "$repository_root" status --porcelain)" ]]; then
  echo "refusing to build an image from a dirty checkout; commit or supply an explicit provenance process" >&2
  exit 1
fi

docker build \
  --file "$repository_root/deploy/images/control-plane/Dockerfile" \
  --label "org.opencontainers.image.revision=$(git -C "$repository_root" rev-parse HEAD)" \
  --tag "$build_image_ref" \
  "$repository_root" >&2

image_ref="$requested_image_ref"
if [[ -z "$image_ref" ]]; then
  image_id="$(docker image inspect "$build_image_ref" --format '{{.Id}}')"
  readonly image_id
  readonly content_id="${image_id#sha256:}"
  image_ref="local/ayni-orchestrator-control-plane:git-${revision}-${content_id:0:12}"
  docker image tag "$build_image_ref" "$image_ref"
  docker image remove "$build_image_ref" >/dev/null
fi
readonly image_ref

docker image inspect "$image_ref" \
  --format 'image={{.Id}} platform={{.Os}}/{{.Architecture}} revision={{index .Config.Labels "org.opencontainers.image.revision"}}' >&2
printf '%s\n' "$image_ref"
