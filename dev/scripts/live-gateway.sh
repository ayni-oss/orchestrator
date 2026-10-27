#!/usr/bin/env bash
set -euo pipefail
readonly dev_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$dev_directory/lib/common.sh"
[[ $# -eq 0 ]] || { echo 'usage: dev/scripts/live-gateway.sh' >&2; exit 2; }
dev_context >/dev/null
export AYNI_ORCHESTRATOR_NAMESPACE="${AYNI_ORCHESTRATOR_NAMESPACE:-ayni-orchestrator-live}"
namespace="$(dev_namespace)"
cd "$DEV_REPOSITORY_ROOT"
cargo build --locked -p ayni-orchestrator-control-plane --bin workflow-gateway
binary="${CARGO_TARGET_DIR:-$DEV_REPOSITORY_ROOT/target}/debug/workflow-gateway"
exec python3 "$dev_directory/lib/gateway.py" "$namespace" "$binary"
