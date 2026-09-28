#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
if [[ $# -ne 0 ]]; then
  echo "usage: $0" >&2
  exit 2
fi
if ! command -v helm >/dev/null 2>&1; then
  echo "Helm is required to deploy the local chart" >&2
  exit 1
fi
context="$($repository_root/scripts/local/require-docker-desktop-context.sh)"
readonly context
namespace="$("$repository_root/scripts/local/resolve-namespace.sh")"
readonly namespace
readonly deployment_profile="${AYNI_ORCHESTRATOR_DEPLOYMENT_PROFILE:-local}"
readonly release_name="ayni-orchestrator"
readonly chart="$repository_root/deploy/charts/local"
case "$deployment_profile" in
  local) readonly custom_clients=false ;;
  custom-clients) readonly custom_clients=true ;;
  *)
    echo "AYNI_ORCHESTRATOR_DEPLOYMENT_PROFILE must be local or custom-clients" >&2
    exit 1
    ;;
esac

helm_release_exists() {
  local release="$1"
  local output
  if output="$(helm status "$release" --kube-context "$context" --namespace "$namespace" 2>&1)"; then
    return 0
  fi
  if grep -q 'release: not found' <<<"$output"; then
    return 1
  fi
  echo "unable to inspect Helm release $release" >&2
  exit 1
}

ownership_args=()
delete_legacy_network_policy=false
if ! helm_release_exists "$release_name"; then
  legacy_resources=false
  inspect_resource() {
    local resource="$1"
    local scope="$2"
    local kubectl_scope=(--context "$context")
    if [[ "$scope" == namespaced ]]; then
      kubectl_scope+=(--namespace "$namespace")
    fi
    if ! kubectl "${kubectl_scope[@]}" get "$resource" >/dev/null 2>&1; then
      return
    fi

    local managed_by owner owner_namespace
    managed_by="$(kubectl "${kubectl_scope[@]}" get "$resource" \
      -o jsonpath='{.metadata.labels.app\.kubernetes\.io/managed-by}')"
    owner="$(kubectl "${kubectl_scope[@]}" get "$resource" \
      -o jsonpath='{.metadata.annotations.meta\.helm\.sh/release-name}')"
    owner_namespace="$(kubectl "${kubectl_scope[@]}" get "$resource" \
      -o jsonpath='{.metadata.annotations.meta\.helm\.sh/release-namespace}')"
    if [[ "$managed_by" == Helm \
      && ( "$owner" != "$release_name" || "$owner_namespace" != "$namespace" ) ]]; then
      if [[ "$resource" == persistentvolumeclaim/managed-check-evidence \
        && "$owner" == ayni-orchestrator-managed-check \
        && "$owner_namespace" == "$namespace" ]]; then
        if helm_release_exists "$owner"; then
          echo "refusing to take $resource while fixture release $owner still exists" >&2
          exit 1
        fi
      else
        echo "refusing to take $resource from Helm release ${owner:-unknown} in namespace ${owner_namespace:-unknown}" >&2
        exit 1
      fi
    fi
    if [[ "$resource" == networkpolicy/custom-client-grpc && "$custom_clients" == false ]]; then
      delete_legacy_network_policy=true
    fi
    legacy_resources=true
  }

  for resource in \
    serviceaccount/control-plane \
    serviceaccount/prometheus \
    serviceaccount/kube-state-metrics \
    configmap/control-plane-config \
    configmap/prometheus-config \
    configmap/grafana-provisioning \
    configmap/grafana-dashboard \
    persistentvolumeclaim/control-plane-state \
    persistentvolumeclaim/development-checkpoints \
    persistentvolumeclaim/managed-check-evidence \
    persistentvolumeclaim/prepared-environments \
    role/control-plane-attempt-jobs \
    role/control-plane-development-clone-secrets \
    role/kube-state-metrics \
    rolebinding/control-plane-attempt-jobs \
    rolebinding/control-plane-development-clone-secrets \
    rolebinding/kube-state-metrics \
    deployment/control-plane \
    deployment/prometheus \
    deployment/kube-state-metrics \
    deployment/grafana \
    service/control-plane \
    service/web \
    service/prometheus \
    service/kube-state-metrics \
    service/grafana \
    networkpolicy/custom-client-grpc; do
    inspect_resource "$resource" namespaced
  done
  readonly prometheus_role="$(printf '%s-prometheus' "$namespace" | cut -c1-63 | sed 's/-*$//')"
  inspect_resource "clusterrole/$prometheus_role" cluster
  inspect_resource "clusterrolebinding/$prometheus_role" cluster

  if [[ "$legacy_resources" == true ]]; then
    echo "adopting legacy local resources into Helm release $release_name"
    kubectl --context "$context" --namespace "$namespace" get pvc \
      control-plane-state development-checkpoints managed-check-evidence prepared-environments \
      --ignore-not-found \
      -o custom-columns=NAME:.metadata.name,UID:.metadata.uid,SIZE:.spec.resources.requests.storage
    ownership_args+=(--take-ownership)
  fi
fi
readonly ownership_args

"$repository_root/scripts/local/check-capacity.sh" 12
control_plane_image_ref="$($repository_root/scripts/local/build-image.sh)"
readonly control_plane_image_ref
worker_image_ref="$($repository_root/scripts/local/build-worker-image.sh)"
readonly worker_image_ref
web_image_ref="$($repository_root/scripts/local/build-web-image.sh)"
readonly web_image_ref
"$repository_root/scripts/local/load-image.sh" "$control_plane_image_ref"
"$repository_root/scripts/local/load-image.sh" "$worker_image_ref"
"$repository_root/scripts/local/load-image.sh" "$web_image_ref"

echo "renderer=$(helm version --short 2>/dev/null)"
# The guarded ownership argument provides the one-way legacy migration only
# when an existing release is absent. Retained claims remain across normal
# release uninstall/reinstall cycles.
helm upgrade --install "$release_name" "$chart" \
  --kube-context "$context" \
  --namespace "$namespace" \
  --create-namespace \
  --force-conflicts \
  "${ownership_args[@]}" \
  --set-string "images.controlPlane=$control_plane_image_ref" \
  --set-string "images.worker=$worker_image_ref" \
  --set-string "images.web=$web_image_ref" \
  --set "customClients.enabled=$custom_clients" \
  --wait \
  --timeout 5m

if [[ "$delete_legacy_network_policy" == true ]]; then
  kubectl --context "$context" --namespace "$namespace" delete \
    networkpolicy/custom-client-grpc --ignore-not-found >/dev/null
fi

pod=""
actual_control_plane_image=""
actual_control_plane_image_id=""
actual_worker_image=""
actual_web_image=""
actual_web_image_id=""
for candidate in $(kubectl --context "$context" --namespace "$namespace" get pods -l app.kubernetes.io/name=ayni-orchestrator-control-plane -o jsonpath='{range .items[*]}{.metadata.name}{"\n"}{end}'); do
  candidate_control_plane_image="$(kubectl --context "$context" --namespace "$namespace" get pod "$candidate" -o jsonpath='{.spec.containers[?(@.name=="control-plane")].image}')"
  candidate_control_plane_image_id="$(kubectl --context "$context" --namespace "$namespace" get pod "$candidate" -o jsonpath='{.status.containerStatuses[?(@.name=="control-plane")].imageID}')"
  candidate_worker_image="$(kubectl --context "$context" --namespace "$namespace" get pod "$candidate" -o jsonpath='{.spec.containers[?(@.name=="control-plane")].env[?(@.name=="ORCHESTRATOR_WORKER_IMAGE")].value}')"
  candidate_web_image="$(kubectl --context "$context" --namespace "$namespace" get pod "$candidate" -o jsonpath='{.spec.containers[?(@.name=="web")].image}')"
  candidate_web_image_id="$(kubectl --context "$context" --namespace "$namespace" get pod "$candidate" -o jsonpath='{.status.containerStatuses[?(@.name=="web")].imageID}')"
  if [[ "$candidate_control_plane_image" == "$control_plane_image_ref" \
    && "$candidate_worker_image" == "$worker_image_ref" \
    && "$candidate_web_image" == "$web_image_ref" \
    && -n "$candidate_control_plane_image_id" \
    && -n "$candidate_web_image_id" ]]; then
    pod="$candidate"
    actual_control_plane_image="$candidate_control_plane_image"
    actual_control_plane_image_id="$candidate_control_plane_image_id"
    actual_worker_image="$candidate_worker_image"
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
"$repository_root/scripts/local/prune-project-images.sh" \
  "$control_plane_image_ref" "$worker_image_ref" "$web_image_ref"
