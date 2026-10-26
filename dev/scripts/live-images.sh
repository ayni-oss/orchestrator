#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/../lib/common.sh"
source "$DEV_DIRECTORY/lib/images.sh"
[[ $# == 2 && "$1" =~ ^(control-plane|conversation-supervisor)$ && "$2" =~ ^[a-f0-9]{64}$ ]] || exit 2
dev_context >/dev/null
dev_check_capacity 8
component="$1"
repository="local/ayni-live-$component"
build_image="$repository:build-$$"
docker build --file "$DEV_DIRECTORY/live.Dockerfile" --build-arg "COMPONENT=$component" --target "$component" \
  --label "build.ayni.dev.source=$2" --tag "$build_image" "$DEV_REPOSITORY_ROOT" >&2
content="$(docker image inspect "$build_image" --format '{{.Id}}')"
[[ "$content" =~ ^sha256:[a-f0-9]{64}$ ]] || { echo 'invalid image content identity' >&2; exit 1; }
image="$repository:${2:0:12}-${content:7:12}"
docker image tag "$build_image" "$image"
docker image remove "$build_image" >/dev/null
dev_load_image "$image"
printf '%s\n' "$image"
