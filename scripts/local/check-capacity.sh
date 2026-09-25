#!/usr/bin/env bash
set -euo pipefail

readonly node_name="desktop-control-plane"
readonly minimum_gib="${1:-8}"
node_id="$(docker inspect "$node_name" --format '{{.Id}}')"
readonly node_id
available_kib="$(docker exec "$node_id" df -Pk /var | awk 'NR==2 {print $4}')"
readonly available_kib
required_kib="$((minimum_gib * 1024 * 1024))"
readonly required_kib
if (( available_kib < required_kib )); then
  printf 'Docker Desktop has less than %s GiB free; refusing a build/deploy that could strand managed Jobs\n' "$minimum_gib" >&2
  docker exec "$node_id" df -h /var >&2
  echo 'Expand Docker Desktop storage or explicitly remove unused project images/build cache; never prune PVC volumes implicitly.' >&2
  exit 1
fi
printf 'Docker Desktop capacity check passed: %s GiB minimum\n' "$minimum_gib" >&2
