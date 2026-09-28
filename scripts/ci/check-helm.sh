#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
readonly chart="$repository_root/deploy/charts/local"
readonly scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/helm-plugins"
export HELM_PLUGINS="$scratch/helm-plugins"

if ! command -v helm >/dev/null 2>&1; then
  echo "Helm is required to validate the local chart" >&2
  exit 1
fi
if find "$repository_root" \
    \( -path "$repository_root/.git" -o -path "$repository_root/.worktrees" \) -prune -o \
    -name kustomization.yaml -print -quit | grep -q .; then
  echo "Kustomize manifests are no longer supported; use deploy/charts/local" >&2
  exit 1
fi

helm lint --strict "$chart"
helm template ayni-orchestrator "$chart" \
  --namespace ayni-orchestrator-dev \
  --set-string images.controlPlane=local/ayni-orchestrator-control-plane:test \
  --set-string images.worker=local/ayni-orchestrator-worker:test \
  --set-string images.web=local/ayni-orchestrator-web:test \
  >"$scratch/local.yaml"
helm template ayni-orchestrator "$chart" \
  --namespace isolated-local \
  --set customClients.enabled=true \
  >"$scratch/custom-clients.yaml"
helm template ayni-orchestrator "$chart" \
  --namespace ayni-orchestrator-dev \
  --set observability.enabled=false \
  >"$scratch/no-observability.yaml"
helm template ayni-orchestrator-managed-check "$chart" \
  --namespace ayni-orchestrator-dev \
  --set core.enabled=false \
  --set observability.enabled=false \
  --set fixtures.managedCheck.enabled=true \
  --set fixtures.managedCheck.createEvidenceClaim=true \
  >"$scratch/managed-check.yaml"
helm template ayni-orchestrator-artifact-rejection "$chart" \
  --namespace ayni-orchestrator-dev \
  --set core.enabled=false \
  --set observability.enabled=false \
  --set fixtures.artifactRejection.enabled=true \
  >"$scratch/artifact-rejection.yaml"
if helm template invalid-fixtures "$chart" \
    --set fixtures.managedCheck.enabled=true \
    --set fixtures.artifactRejection.enabled=true \
    >"$scratch/invalid-fixtures.yaml" 2>/dev/null; then
  echo "chart accepted mutually exclusive managed-check fixtures" >&2
  exit 1
fi
if helm template duplicate-claim "$chart" \
    --set fixtures.managedCheck.enabled=true \
    --set fixtures.managedCheck.createEvidenceClaim=true \
    >"$scratch/duplicate-claim.yaml" 2>/dev/null; then
  echo "chart accepted duplicate managed-check evidence claims" >&2
  exit 1
fi

grep -q '^  ORCHESTRATOR_KUBERNETES_NAMESPACE: "ayni-orchestrator-dev"$' "$scratch/local.yaml"
grep -q '^kind: Deployment$' "$scratch/local.yaml"
grep -q '^  name: prometheus$' "$scratch/local.yaml"
grep -q '^  name: grafana$' "$scratch/local.yaml"
grep -q '^  name: grafana-dashboard$' "$scratch/local.yaml"
grep -q '^  name: custom-client-grpc$' "$scratch/custom-clients.yaml"
grep -q 'namespace=\\"isolated-local\\"' "$scratch/custom-clients.yaml"
if grep -Eq '__AYNI_NAMESPACE__|namespace=\\"ayni-orchestrator-dev\\"' "$scratch/custom-clients.yaml"; then
  echo "isolated namespace was not propagated into the Grafana dashboard" >&2
  exit 1
fi
grep -q '^  name: managed-check$' "$scratch/managed-check.yaml"
grep -q '^  name: managed-check-evidence$' "$scratch/managed-check.yaml"
grep -q '^  name: managed-check-artifact-rejection$' "$scratch/artifact-rejection.yaml"
grep -q '^  name: ayni-orchestrator-dev-prometheus$' "$scratch/local.yaml"
[[ "$(grep -c '^    helm.sh/resource-policy: keep$' "$scratch/local.yaml")" -eq 4 ]]
for claim in control-plane-state development-checkpoints managed-check-evidence prepared-environments; do
  grep -q "^  name: $claim$" "$scratch/local.yaml"
done
if grep -Eq '^kind: Ingress$|^  type: (LoadBalancer|NodePort)$|nodes/proxy' "$scratch/local.yaml"; then
  echo "local chart rendered a forbidden public service or node-proxy permission" >&2
  exit 1
fi
if grep -Eq '^  name: (prometheus|grafana|kube-state-metrics)$' "$scratch/no-observability.yaml"; then
  echo "observability-disabled render still contains monitoring resources" >&2
  exit 1
fi
if grep -q '^  name: control-plane$' "$scratch/managed-check.yaml"; then
  echo "managed-check fixture unexpectedly rendered the core release" >&2
  exit 1
fi
