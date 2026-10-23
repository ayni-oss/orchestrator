#!/usr/bin/env bash
set -euo pipefail
readonly dev_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$dev_directory/lib/common.sh"
source "$dev_directory/lib/images.sh"
[[ $# -eq 0 ]] || { echo 'usage: dev/ctl quality-runtime' >&2; exit 2; }
dev_context >/dev/null
dev_check_capacity 4
readonly image=local/ayni-task-quality:acceptance
readonly cli_version="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["cli_version"])' "$DEV_REPOSITORY_ROOT/scripts/ci/ayni-tool.json")"
[[ "$(ayni --version)" == "ayni $cli_version" ]] || {
  echo "quality-runtime requires the pinned Ayni $cli_version CLI" >&2; exit 2;
}
readonly builder="ghcr.io/ayni-oss/ayni-builder:$cli_version"
# An explicit current executor also handles a retained build record from the
# preceding CLI version; Ayni still verifies the supplied image itself.
builder_digest="$(docker buildx imagetools inspect "$builder" --format '{{json .Manifest}}' \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["digest"])')"
[[ "$builder_digest" =~ ^sha256:[a-f0-9]{64}$ ]] || { echo 'builder digest missing' >&2; exit 1; }
# No credentials/signing material are read or injected into this fixture build.
ayni env build --repo-root "$dev_directory/fixtures/task-quality/source" \
  --executor-image "$builder@$builder_digest" --tag "$image" >&2
dev_load_image "$image"
# Docker Desktop imports the archive's immutable OCI manifest. Bind the prepared
# attempt to that digest rather than the mutable convenience tag.
digest="$(docker exec desktop-control-plane ctr --namespace k8s.io images list \
  | awk '$1=="docker.io/local/ayni-task-quality:acceptance" {print $3}')"
[[ "$digest" =~ ^sha256:[a-f0-9]{64}$ ]] || { echo 'managed fixture manifest identity missing' >&2; exit 1; }
readonly runtime="docker.io/local/ayni-task-quality@$digest"
# A digest alias makes kubelet resolve the already imported local image without
# requiring a registry. This changes a local containerd reference, not its bytes.
docker exec desktop-control-plane ctr --namespace k8s.io images tag --force \
  docker.io/local/ayni-task-quality:acceptance "$runtime" >&2
mkdir -p "$DEV_REPOSITORY_ROOT/.ayni"
python3 - "$runtime" "$DEV_REPOSITORY_ROOT/.ayni/task-quality-runtime.json" <<'PY'
import json, pathlib, sys
pathlib.Path(sys.argv[2]).write_text(json.dumps({'runtime_image':sys.argv[1]},indent=2)+'\n')
PY
printf '%s\n' "$runtime"
