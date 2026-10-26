#!/usr/bin/env bash
set -euo pipefail
readonly dev_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$dev_directory/lib/common.sh"
source "$dev_directory/lib/images.sh"
readonly context="$(dev_context)" namespace="$(dev_namespace)"
[[ $# -eq 1 && "$1" == /* && ! -e "$1" && "$namespace" == ayni-orchestrator-acceptance-* ]] || {
  echo 'usage: dev/scripts/dependency-acceptance.sh ABSOLUTE_NEW_EVIDENCE_DIRECTORY' >&2; exit 2;
}
mkdir -p "$1"
readonly evidence="$1"
cd "$DEV_REPOSITORY_ROOT"
cargo build --locked -p ayni-orchestrator-control-plane \
  --bin workflow-gateway --bin workflow-prepare-task --bin workflow-publish-task > "$evidence/host-helper-build.txt" 2>&1
runtime="${AYNI_ORCHESTRATOR_QUALITY_RUNTIME_IMAGE:-$(python3 -c 'import json; print(json.load(open(".ayni/task-quality-runtime.json"))["runtime_image"])')}"
[[ "$runtime" =~ @sha256:[a-f0-9]{64}$ ]] || { echo 'digest-pinned task-quality runtime required' >&2; exit 1; }
fixture_image="$(dev_build_image publication-fixture)"
dev_load_image "$fixture_image"
node "$dev_directory/lib/dependency-acceptance.mjs" "$DEV_REPOSITORY_ROOT" "$evidence" "$namespace" "$runtime" "$fixture_image"
