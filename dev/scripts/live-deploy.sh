#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/../lib/common.sh"
export HELM_NO_PLUGINS=1
[[ $# == 2 && "$1" == local/ayni-live-control-plane:* && "$2" == local/ayni-live-conversation-supervisor:* ]] || exit 2
context="$(dev_context)"
namespace="$(dev_namespace)"
dev_check_component_names "$context" "$namespace"
[[ "$(helm version --template '{{ .Version }}')" == v4.1.1* ]] || { echo 'Helm 4.1.1 required' >&2; exit 1; }
"$DEV_DIRECTORY/scripts/configure-development-secrets.sh"
dev_build_chart_dependencies
# Explicit migrations quiesce the serving control plane before its migration
# init container can alter the schema. Workers retain identity and retry reports.
restore_serving=false
if [[ "${AYNI_ORCHESTRATOR_LIVE_MIGRATE:-false}" == true ]]; then
  existing="$(kubectl --context "$context" --namespace "$namespace" get deployment/control-plane --ignore-not-found -o name)"
  if [[ -n "$existing" ]]; then
    restore_serving=true
    kubectl --context "$context" --namespace "$namespace" scale deployment/control-plane --replicas=0
    kubectl --context "$context" --namespace "$namespace" wait --for=delete pod \
      -l app.kubernetes.io/name=ayni-orchestrator-control-plane --timeout=2m
  fi
fi
# Reuse provider/fixture grants on subsequent reloads; never reset them implicitly.
helm upgrade --install "$DEV_RELEASE_NAME" "$DEV_CHART" --reuse-values \
  --kube-context "$context" --namespace "$namespace" \
  --set-string "ayni-orchestrator.images.controlPlane=$1" \
  --set-string "ayni-orchestrator.images.conversationSupervisor=$2" \
  --timeout 5m
if [[ "$restore_serving" == true ]]; then
  # The local profile is a singleton. A prior failed migration may have left
  # observed replicas at zero; never use that as the desired serving count.
  kubectl --context "$context" --namespace "$namespace" scale deployment/control-plane --replicas=1
fi
for workload in statefulset/postgres statefulset/jetstream deployment/control-plane statefulset/conversation-supervisor deployment/task-supervisor; do
  kubectl --context "$context" --namespace "$namespace" rollout status "$workload" --timeout=5m
done

if [[ -n "$(kubectl --context "$context" --namespace "$namespace" get deployment/task-publisher --ignore-not-found -o name)" ]]; then
  kubectl --context "$context" --namespace "$namespace" rollout status deployment/task-publisher --timeout=5m
fi
ready="$(kubectl --context "$context" --namespace "$namespace" get deployment/control-plane -o jsonpath='{.status.readyReplicas}')"
[[ "$ready" == 1 ]] || { echo 'live control plane is not serving one ready replica' >&2; exit 1; }
