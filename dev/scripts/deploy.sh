#!/usr/bin/env bash
set -euo pipefail

readonly dev_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$dev_directory/lib/common.sh"
source "$dev_directory/lib/images.sh"
# Local deployment uses no Helm plugins or encrypted Helm values.
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
development_mode="${AYNI_ORCHESTRATOR_DEVELOPMENT_MODE:-standard}"
case "$development_mode" in
  standard)
    control_plane_component=control-plane
    web_component=web
    hot_reload_values=()
    ;;
  hot-reload)
    control_plane_component=control-plane-dev
    web_component=web-dev
    hot_reload_values=(--values "$DEV_DIRECTORY/charts/ayni-orchestrator-local/values-hot-reload.yaml")
    ;;
  *)
    echo "AYNI_ORCHESTRATOR_DEVELOPMENT_MODE must be standard or hot-reload" >&2
    exit 1
    ;;
esac
deployment_profile="${AYNI_ORCHESTRATOR_DEPLOYMENT_PROFILE:-local}"
case "$deployment_profile" in
  local) custom_clients=false ;;
  custom-clients) custom_clients=true ;;
  *)
    echo "AYNI_ORCHESTRATOR_DEPLOYMENT_PROFILE must be local or custom-clients" >&2
    exit 1
    ;;
esac

dev_build_chart_dependencies
dev_check_capacity 12
control_plane_image_ref="$(dev_build_image "$control_plane_component")"
worker_image_ref="$(dev_build_image worker)"
web_image_ref="$(dev_build_image "$web_component")"
dev_load_image "$control_plane_image_ref"
dev_load_image "$worker_image_ref"
dev_load_image "$web_image_ref"
runtime_worker_image="$(dev_pin_worker_image "$worker_image_ref")"
connection_test_image="$(dev_pin_worker_image "$control_plane_image_ref")"
# Register only the explicitly provisioned local signing Secret. Request names,
# never credential values; an installation without it starts with UI setup.
admin_certificates='[]'
signing_secret_name="$(kubectl --context "$context" --namespace "$namespace" \
  get secret environment-producer --ignore-not-found -o name)"
if [[ -n "$signing_secret_name" ]]; then
  admin_certificates='[{"id":"certificate-admin-environment-producer","displayName":"Local environment signing","secretName":"environment-producer"}]'
fi

echo "renderer=$(helm version --short 2>/dev/null)"
helm upgrade --install "$DEV_RELEASE_NAME" "$DEV_CHART" --reset-values \
  --kube-context "$context" \
  --namespace "$namespace" \
  --create-namespace \
  "${hot_reload_values[@]}" \
  --set-string "ayni-orchestrator.images.controlPlane=$control_plane_image_ref" \
  --set-string "ayni-orchestrator.images.web=$web_image_ref" \
  --set-string "ayni-orchestrator.connections.testImage=$connection_test_image" \
  --set-string "ayni-orchestrator.connections.githubAppSecretName=${AYNI_ORCHESTRATOR_GITHUB_APP_SECRET_NAME:-}" \
  --set-json "ayni-orchestrator.connections.adminCertificates=$admin_certificates" \
  --set "ayni-orchestrator.customClients.enabled=$custom_clients" \
  --set ayni-orchestrator.runtime.enabled=true \
  --set-string "ayni-orchestrator.runtime.workerImage=$runtime_worker_image" \
  --wait=hookOnly \
  --timeout 5m

# Runtime output storage binds when the first worker Job consumes it.
# Wait for the application stack without requiring that claim to bind yet.
for deployment in control-plane prometheus grafana otel-collector jaeger; do
  kubectl --context "$context" --namespace "$namespace" rollout status \
    "deployment/$deployment" --timeout=5m
done
kubectl --context "$context" --namespace "$namespace" rollout status \
  statefulset/jetstream --timeout=5m

pod=""
actual_control_plane_image=""
actual_control_plane_image_id=""
actual_worker_image="$(kubectl --context "$context" --namespace "$namespace" get configmap runtime-config -o jsonpath='{.data.runtime\.json}' | python3 -c 'import json,sys; print(json.load(sys.stdin)["worker_image"])')"
if [[ "$actual_worker_image" != "$runtime_worker_image" ]]; then
  echo "runtime configuration does not reference the deployed worker digest" >&2
  exit 1
fi
actual_web_image=""
actual_web_image_id=""
for candidate in $(kubectl --context "$context" --namespace "$namespace" get pods \
  -l app.kubernetes.io/name=ayni-orchestrator-control-plane \
  -o jsonpath='{range .items[*]}{.metadata.name}{"\n"}{end}'); do
  candidate_control_plane_image="$(kubectl --context "$context" --namespace "$namespace" get pod "$candidate" -o jsonpath='{.spec.containers[?(@.name=="control-plane")].image}')"
  candidate_control_plane_image_id="$(kubectl --context "$context" --namespace "$namespace" get pod "$candidate" -o jsonpath='{.status.containerStatuses[?(@.name=="control-plane")].imageID}')"
  candidate_web_image="$(kubectl --context "$context" --namespace "$namespace" get pod "$candidate" -o jsonpath='{.spec.containers[?(@.name=="web")].image}')"
  candidate_web_image_id="$(kubectl --context "$context" --namespace "$namespace" get pod "$candidate" -o jsonpath='{.status.containerStatuses[?(@.name=="web")].imageID}')"
  if [[ "$candidate_control_plane_image" == "$control_plane_image_ref" \
    && "$candidate_web_image" == "$web_image_ref" \
    && -n "$candidate_control_plane_image_id" \
    && -n "$candidate_web_image_id" ]]; then
    pod="$candidate"
    actual_control_plane_image="$candidate_control_plane_image"
    actual_control_plane_image_id="$candidate_control_plane_image_id"
    actual_web_image="$candidate_web_image"
    actual_web_image_id="$candidate_web_image_id"
    break
  fi
done

if [[ -z "$pod" ]]; then
  echo "no active pod reported the deployed control-plane, worker, and web image identities" >&2
  exit 1
fi

echo "deployed pod=$pod control_plane_image=$actual_control_plane_image control_plane_imageID=$actual_control_plane_image_id worker_image=$actual_worker_image web_image=$actual_web_image web_imageID=$actual_web_image_id"
dev_prune_images "$control_plane_image_ref" "$worker_image_ref" "$web_image_ref" "$runtime_worker_image" "$connection_test_image"
