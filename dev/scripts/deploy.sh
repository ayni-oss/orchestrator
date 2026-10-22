#!/usr/bin/env bash
set -euo pipefail

readonly dev_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$dev_directory/lib/common.sh"
source "$dev_directory/lib/images.sh"
export HELM_NO_PLUGINS=1

if [[ $# -ne 0 ]]; then
  echo "usage: dev/ctl deploy" >&2
  exit 2
fi
if ! command -v helm >/dev/null 2>&1; then
  echo "Helm is required to deploy the local chart" >&2
  exit 1
fi
helm_version="$(helm version --template '{{ .Version }}' 2>/dev/null)"
if [[ "${helm_version%%+*}" != v4.1.1 ]]; then
  echo "Helm 4.1.1 is required by the local development workflow; found ${helm_version:-unknown}" >&2
  exit 1
fi

context="$(dev_context)"
namespace="$(dev_namespace)"
"$dev_directory/scripts/configure-development-secrets.sh"
dev_build_chart_dependencies
dev_check_capacity 8
task_fixture="${AYNI_ORCHESTRATOR_TASK_FIXTURE:-false}"
[[ "$task_fixture" == true || "$task_fixture" == false ]] || { echo 'AYNI_ORCHESTRATOR_TASK_FIXTURE must be true or false' >&2; exit 2; }
control_plane_image_ref="$(dev_build_image control-plane)"
executor_image_ref="$(dev_build_image scripted-executor)"
dev_load_image "$control_plane_image_ref"
dev_load_image "$executor_image_ref"

echo "renderer=$(helm version --short 2>/dev/null)"
helm upgrade --install "$DEV_RELEASE_NAME" "$DEV_CHART" --reset-values \
  --kube-context "$context" \
  --namespace "$namespace" \
  --set "ayni-orchestrator.taskExecution.fixtureEnabled=$task_fixture" \
  --set-string "ayni-orchestrator.images.controlPlane=$control_plane_image_ref" \
  --set-string "ayni-orchestrator.images.scriptedExecutor=$executor_image_ref" \
  --wait \
  --timeout 5m

for workload in \
  statefulset/workflow-postgres \
  statefulset/workflow-jetstream \
  statefulset/scripted-executor \
  deployment/control-plane \
  deployment/task-supervisor; do
  kubectl --context "$context" --namespace "$namespace" rollout status \
    "$workload" --timeout=5m
done

verify_workload_image() {
  local selector="$1" container="$2" expected="$3" pod actual image_id
  pod="$(kubectl --context "$context" --namespace "$namespace" get pods \
    -l "app.kubernetes.io/name=$selector" -o jsonpath='{.items[0].metadata.name}')"
  actual="$(kubectl --context "$context" --namespace "$namespace" get pod "$pod" \
    -o "jsonpath={.spec.containers[?(@.name==\"$container\")].image}")"
  image_id="$(kubectl --context "$context" --namespace "$namespace" get pod "$pod" \
    -o "jsonpath={.status.containerStatuses[?(@.name==\"$container\")].imageID}")"
  if [[ "$actual" != "$expected" || -z "$image_id" ]]; then
    echo "the ready $container pod does not report the deployed image identity" >&2
    exit 1
  fi
  echo "deployed pod=$pod image=$actual imageID=$image_id"
}

verify_workload_image ayni-orchestrator-control-plane control-plane "$control_plane_image_ref"
verify_workload_image ayni-orchestrator-scripted-executor scripted-executor "$executor_image_ref"
dev_prune_images control-plane "$control_plane_image_ref"
dev_prune_images scripted-executor "$executor_image_ref"
