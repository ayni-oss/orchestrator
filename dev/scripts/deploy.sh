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
helm_version="$(helm version --template '{{ .Version }}' 2>/dev/null)"
readonly helm_version
if [[ "${helm_version%%+*}" != v4.1.1 ]]; then
  echo "Helm 4.1.1 is required for guarded server-side ownership migration; found ${helm_version:-unknown}" >&2
  exit 1
fi
"$repository_root/dev/scripts/build-chart-dependencies.sh"
context="$($repository_root/dev/scripts/require-docker-desktop-context.sh)"
readonly context
namespace="$("$repository_root/dev/scripts/resolve-namespace.sh")"
readonly namespace
readonly deployment_profile="${AYNI_ORCHESTRATOR_DEPLOYMENT_PROFILE:-local}"
readonly release_name="ayni-orchestrator"
readonly chart="$repository_root/dev/charts/ayni-orchestrator-local"
readonly adopt_unlabeled_legacy="${AYNI_ORCHESTRATOR_ADOPT_UNLABELED_LEGACY_RESOURCES:-false}"
case "$adopt_unlabeled_legacy" in
  true | false) ;;
  *)
    echo "AYNI_ORCHESTRATOR_ADOPT_UNLABELED_LEGACY_RESOURCES must be true or false" >&2
    exit 1
    ;;
esac
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
release_installed=false
if helm_release_exists "$release_name"; then
  release_installed=true
fi
if [[ "$release_installed" == false ]]; then
  legacy_resources=false
  inspect_resource() {
    local resource="$1"
    local scope="$2"
    local expected_part="$3"
    local kubectl_scope=(--context "$context")
    if [[ "$scope" == namespaced ]]; then
      kubectl_scope+=(--namespace "$namespace")
    fi
    local resource_name
    if ! resource_name="$(kubectl "${kubectl_scope[@]}" get "$resource" \
      --ignore-not-found -o name)"; then
      echo "unable to inspect $resource before ownership migration" >&2
      exit 1
    fi
    if [[ -z "$resource_name" ]]; then
      return
    fi

    local managed_by owner owner_namespace part_of legacy_signature signature_value
    managed_by="$(kubectl "${kubectl_scope[@]}" get "$resource" \
      -o jsonpath='{.metadata.labels.app\.kubernetes\.io/managed-by}')"
    owner="$(kubectl "${kubectl_scope[@]}" get "$resource" \
      -o jsonpath='{.metadata.annotations.meta\.helm\.sh/release-name}')"
    owner_namespace="$(kubectl "${kubectl_scope[@]}" get "$resource" \
      -o jsonpath='{.metadata.annotations.meta\.helm\.sh/release-namespace}')"
    part_of="$(kubectl "${kubectl_scope[@]}" get "$resource" \
      -o jsonpath='{.metadata.labels.app\.kubernetes\.io/part-of}')"

    # Four superseded Kustomize objects predate common labels. Accept only
    # their exact, bounded identity; all other takeover candidates must carry
    # the expected Ayni ownership label.
    legacy_signature=false
    signature_value=""
    case "$resource" in
      configmap/control-plane-config)
        if ! signature_value="$(kubectl "${kubectl_scope[@]}" get "$resource" \
          -o jsonpath='{.data.ORCHESTRATOR_DATABASE_PATH}')"; then
          echo "unable to verify the legacy signature for $resource" >&2
          exit 1
        fi
        if [[ "$signature_value" == /data/orchestrator.sqlite ]]; then
          legacy_signature=true
        fi
        ;;
      role/control-plane-development-clone-secrets)
        if ! signature_value="$(kubectl "${kubectl_scope[@]}" get "$resource" \
          -o jsonpath='{.rules[0].resources[0]}:{.rules[0].verbs[0]}:{.rules[0].verbs[1]}:{.rules[0].verbs[2]}:{.rules[0].verbs[3]}')"; then
          echo "unable to verify the legacy signature for $resource" >&2
          exit 1
        fi
        if [[ "$signature_value" == secrets:create:get:patch:delete ]]; then
          legacy_signature=true
        fi
        ;;
      rolebinding/control-plane-development-clone-secrets)
        if ! signature_value="$(kubectl "${kubectl_scope[@]}" get "$resource" \
          -o jsonpath='{.roleRef.kind}:{.roleRef.name}:{.subjects[0].kind}:{.subjects[0].name}')"; then
          echo "unable to verify the legacy signature for $resource" >&2
          exit 1
        fi
        if [[ "$signature_value" == Role:control-plane-development-clone-secrets:ServiceAccount:control-plane ]]; then
          legacy_signature=true
        fi
        ;;
      networkpolicy/custom-client-grpc)
        if ! signature_value="$(kubectl "${kubectl_scope[@]}" get "$resource" \
          -o jsonpath='{.spec.podSelector.matchLabels.app\.kubernetes\.io/name}:{.spec.policyTypes[0]}:{.spec.ingress[1].ports[0].port}')"; then
          echo "unable to verify the legacy signature for $resource" >&2
          exit 1
        fi
        if [[ "$signature_value" == ayni-orchestrator-control-plane:Ingress:50052 ]]; then
          legacy_signature=true
        fi
        ;;
    esac
    if [[ "$part_of" != "$expected_part" ]]; then
      if [[ "$legacy_signature" != true || "$adopt_unlabeled_legacy" != true ]]; then
        echo "refusing to take unknown $resource without app.kubernetes.io/part-of=$expected_part" >&2
        if [[ "$legacy_signature" == true ]]; then
          echo "inspect the legacy object, then set AYNI_ORCHESTRATOR_ADOPT_UNLABELED_LEGACY_RESOURCES=true for this one-way migration" >&2
        fi
        exit 1
      fi
    fi
    if [[ -n "$managed_by" && "$managed_by" != Helm ]]; then
      echo "refusing to take $resource managed by $managed_by" >&2
      exit 1
    fi
    if [[ ( -n "$owner" || -n "$owner_namespace" || "$managed_by" == Helm ) \
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
    configmap/control-plane-config \
    persistentvolumeclaim/control-plane-state \
    persistentvolumeclaim/development-checkpoints \
    persistentvolumeclaim/managed-check-evidence \
    persistentvolumeclaim/prepared-environments \
    role/control-plane-attempt-jobs \
    role/control-plane-development-clone-secrets \
    rolebinding/control-plane-attempt-jobs \
    rolebinding/control-plane-development-clone-secrets \
    deployment/control-plane \
    service/control-plane \
    service/web \
    networkpolicy/custom-client-grpc; do
    inspect_resource "$resource" namespaced ayni-orchestrator
  done
  for resource in \
    serviceaccount/prometheus \
    serviceaccount/kube-state-metrics \
    configmap/prometheus-config \
    configmap/ayni-orchestrator-prometheus-config \
    configmap/grafana-provisioning \
    configmap/grafana \
    configmap/grafana-dashboard \
    role/kube-state-metrics \
    rolebinding/kube-state-metrics \
    deployment/prometheus \
    deployment/kube-state-metrics \
    deployment/grafana \
    service/prometheus \
    service/kube-state-metrics \
    service/grafana; do
    inspect_resource "$resource" namespaced ayni-orchestrator-observability
  done
  readonly prometheus_role="$(printf '%s-prometheus' "$namespace" | cut -c1-63 | sed 's/-*$//')"
  inspect_resource "clusterrole/$prometheus_role" cluster ayni-orchestrator-observability
  inspect_resource "clusterrolebinding/$prometheus_role" cluster ayni-orchestrator-observability

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

# Decision 0006 initially shipped hand-written monitoring Deployments. The
# dependency charts use different immutable selectors, so replace only verified
# disposable objects during the one-way dependency-chart migration.
legacy_observability_deployments=()
for deployment in prometheus kube-state-metrics grafana; do
  resource="deployment/$deployment"
  if ! resource_name="$(kubectl --context "$context" --namespace "$namespace" get "$resource" \
    --ignore-not-found -o name)"; then
    echo "unable to inspect $resource before monitoring migration" >&2
    exit 1
  fi
  if [[ -z "$resource_name" ]]; then
    continue
  fi
  if ! selector_identity="$(kubectl --context "$context" --namespace "$namespace" get "$resource" \
    -o go-template='{{len .spec.selector.matchLabels}}:{{if .spec.selector.matchExpressions}}{{len .spec.selector.matchExpressions}}{{else}}0{{end}}:{{index .spec.selector.matchLabels "app.kubernetes.io/name"}}')"; then
    echo "unable to inspect the exact selector for $resource" >&2
    exit 1
  fi
  case "$deployment:$selector_identity" in
    prometheus:1:0:ayni-orchestrator-prometheus | \
      kube-state-metrics:1:0:ayni-orchestrator-kube-state-metrics | \
      grafana:1:0:ayni-orchestrator-grafana) ;;
    *) continue ;;
  esac
  part_of="$(kubectl --context "$context" --namespace "$namespace" get "$resource" \
    -o jsonpath='{.metadata.labels.app\.kubernetes\.io/part-of}')"
  managed_by="$(kubectl --context "$context" --namespace "$namespace" get "$resource" \
    -o jsonpath='{.metadata.labels.app\.kubernetes\.io/managed-by}')"
  owner="$(kubectl --context "$context" --namespace "$namespace" get "$resource" \
    -o jsonpath='{.metadata.annotations.meta\.helm\.sh/release-name}')"
  if [[ "$part_of" != ayni-orchestrator-observability ]]; then
    echo "refusing to replace unknown $resource" >&2
    exit 1
  fi
  if [[ "$release_installed" == true \
    && ( "$managed_by" != Helm || "$owner" != "$release_name" ) ]]; then
    echo "refusing to replace $resource owned by ${owner:-unknown}" >&2
    exit 1
  fi
  legacy_observability_deployments+=("$resource")
done
readonly legacy_observability_deployments

obsolete_observability_configmaps=()
for configmap in prometheus-config grafana-provisioning; do
  resource="configmap/$configmap"
  if ! resource_name="$(kubectl --context "$context" --namespace "$namespace" get "$resource" \
    --ignore-not-found -o name)"; then
    echo "unable to inspect $resource before monitoring migration" >&2
    exit 1
  fi
  if [[ -z "$resource_name" ]]; then
    continue
  fi
  part_of="$(kubectl --context "$context" --namespace "$namespace" get "$resource" \
    -o jsonpath='{.metadata.labels.app\.kubernetes\.io/part-of}')"
  managed_by="$(kubectl --context "$context" --namespace "$namespace" get "$resource" \
    -o jsonpath='{.metadata.labels.app\.kubernetes\.io/managed-by}')"
  owner="$(kubectl --context "$context" --namespace "$namespace" get "$resource" \
    -o jsonpath='{.metadata.annotations.meta\.helm\.sh/release-name}')"
  if [[ "$part_of" != ayni-orchestrator-observability ]]; then
    echo "refusing to replace unknown $resource" >&2
    exit 1
  fi
  if [[ "$release_installed" == true \
    && ( "$managed_by" != Helm || "$owner" != "$release_name" ) ]]; then
    echo "refusing to replace $resource owned by ${owner:-unknown}" >&2
    exit 1
  fi
  obsolete_observability_configmaps+=("$resource")
done
readonly obsolete_observability_configmaps

"$repository_root/dev/scripts/check-capacity.sh" 12
control_plane_image_ref="$($repository_root/dev/scripts/build-image.sh control-plane)"
readonly control_plane_image_ref
worker_image_ref="$($repository_root/dev/scripts/build-image.sh worker)"
readonly worker_image_ref
web_image_ref="$($repository_root/dev/scripts/build-image.sh web)"
readonly web_image_ref
"$repository_root/dev/scripts/load-image.sh" "$control_plane_image_ref"
"$repository_root/dev/scripts/load-image.sh" "$worker_image_ref"
"$repository_root/dev/scripts/load-image.sh" "$web_image_ref"

echo "renderer=$(helm version --short 2>/dev/null)"
if (( ${#legacy_observability_deployments[@]} > 0 )); then
  echo "replacing verified legacy monitoring Deployments with dependency-chart resources"
  kubectl --context "$context" --namespace "$namespace" delete \
    "${legacy_observability_deployments[@]}" --ignore-not-found >/dev/null
fi
if (( ${#obsolete_observability_configmaps[@]} > 0 )); then
  kubectl --context "$context" --namespace "$namespace" delete \
    "${obsolete_observability_configmaps[@]}" --ignore-not-found >/dev/null
fi
# The guarded ownership argument provides the one-way legacy migration only
# when an existing release is absent. Retained claims remain across normal
# release uninstall/reinstall cycles.
helm upgrade --install "$release_name" "$chart" \
  --kube-context "$context" \
  --namespace "$namespace" \
  --create-namespace \
  --force-conflicts \
  "${ownership_args[@]}" \
  --set-string "ayni-orchestrator.images.controlPlane=$control_plane_image_ref" \
  --set-string "ayni-orchestrator.images.worker=$worker_image_ref" \
  --set-string "ayni-orchestrator.images.web=$web_image_ref" \
  --set "ayni-orchestrator.customClients.enabled=$custom_clients" \
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
"$repository_root/dev/scripts/prune-project-images.sh" \
  "$control_plane_image_ref" "$worker_image_ref" "$web_image_ref"
