#!/usr/bin/env bash
# Metadata-only real Kubernetes list/watch/deletion/reconnect test; no workload runs.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
namespace="${ORCHESTRATOR_WATCH_TEST_NAMESPACE:-ayni-watch-test-$$}"
kubectl --context docker-desktop get namespace "$namespace" >/dev/null 2>&1 && {
  echo "Refusing to reuse existing namespace $namespace" >&2
  exit 1
}
kubectl --context docker-desktop create namespace "$namespace"
trap 'kubectl --context docker-desktop delete namespace "$namespace" --wait=false >/dev/null' EXIT
cd "$root"
ORCHESTRATOR_WATCH_TEST_NAMESPACE="$namespace" cargo test --locked -p ayni-orchestrator-kubernetes docker_desktop_watch -- --ignored --nocapture
