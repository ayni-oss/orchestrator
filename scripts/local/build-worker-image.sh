#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
readonly revision="$(git -C "$repository_root" rev-parse --short=12 HEAD)"
readonly image_ref="${1:-local/ayni-orchestrator-worker:git-${revision}}"

if [[ -n "$(git -C "$repository_root" status --porcelain)" ]]; then
  echo "refusing to build an image from a dirty checkout; commit or supply an explicit provenance process" >&2
  exit 1
fi

docker build \
  --file "$repository_root/deploy/images/worker/Dockerfile" \
  --label "org.opencontainers.image.revision=$(git -C "$repository_root" rev-parse HEAD)" \
  --tag "$image_ref" \
  "$repository_root" >&2
docker image inspect "$image_ref" \
  --format 'image={{.Id}} platform={{.Os}}/{{.Architecture}} revision={{index .Config.Labels "org.opencontainers.image.revision"}}' >&2
printf '%s\n' "$image_ref"
