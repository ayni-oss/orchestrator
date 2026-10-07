#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
readonly application_chart="$repository_root/deploy/charts/ayni-orchestrator"
readonly local_chart="$repository_root/dev/charts/ayni-orchestrator-local"
readonly scratch="$(mktemp -d)"
trap 'rm -rf "$scratch" "$local_chart/charts"' EXIT
mkdir -p "$scratch/helm-plugins" "$scratch/helm-cache"
export HELM_PLUGINS="$scratch/helm-plugins"
export HELM_REPOSITORY_CONFIG="$scratch/helm-repositories.yaml"
export HELM_REPOSITORY_CACHE="$scratch/helm-cache"

if ! command -v helm >/dev/null 2>&1; then
  echo "Helm is required to validate the deployment charts" >&2
  exit 1
fi
bash -n \
  "$repository_root/dev/ctl" \
  "$repository_root"/dev/lib/*.sh \
  "$repository_root"/dev/scripts/*.sh \
  "$repository_root"/dev/tests/*.sh
"$repository_root/dev/ctl" help >/dev/null
if find "$repository_root" \
    \( -path "$repository_root/.git" -o -path "$repository_root/.worktrees" \) -prune -o \
    -name kustomization.yaml -print -quit | grep -q .; then
  echo "Kustomize manifests are no longer supported; use deploy/charts" >&2
  exit 1
fi

rm -rf "$local_chart/charts"
helm repo add prometheus-community https://prometheus-community.github.io/helm-charts >/dev/null
helm repo add grafana https://grafana.github.io/helm-charts >/dev/null
helm dependency build "$local_chart"
helm lint --strict "$application_chart" --set runtime.enabled=false
helm lint --strict "$application_chart" --set runtime.enabled=true \
  --set-string runtime.workerImage=registry.example/worker@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
helm lint --strict "$local_chart" --set ayni-orchestrator.runtime.enabled=false

helm template reusable "$application_chart" --set runtime.enabled=false \
  --namespace reusable-install \
  --set-string images.controlPlane=registry.example/control-plane:test \
  --set-string images.web=registry.example/web:test \
  >"$scratch/application.yaml"
helm template runtime "$application_chart" --namespace runtime-test \
  --set runtime.enabled=true \
  --set-string runtime.workerImage=registry.example/worker@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
  --set-string 'runtime.serviceAccounts.imagePullSecrets[0].name=registry-pull' \
  >"$scratch/runtime.yaml"
helm template runtime "$application_chart" --namespace runtime-test \
  --set runtime.enabled=true --set runtime.serviceAccounts.create=false \
  --set-string runtime.workerImage=registry.example/worker@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
  >"$scratch/runtime-existing-accounts.yaml"
# Managed onboarding inherits deployment policy without a repository allowlist.
helm template runtime-defaults "$application_chart" --namespace runtime-test \
  --set runtime.enabled=true \
  --set-string runtime.workerImage=registry.example/worker@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
  --set-string runtime.managedRuntime.platform=linux/amd64 \
  --set-string runtime.managedRuntime.pull_secret=shared-runtime-pull \
  >"$scratch/runtime-defaults.yaml"
grep -Fq '\"managed_runtime\":{' "$scratch/runtime-defaults.yaml"
grep -Fq '\"pull_secret\":\"shared-runtime-pull\"' "$scratch/runtime-defaults.yaml"
for account in runtime-coding runtime-builder; do
  grep -q "^  name: $account$" "$scratch/runtime.yaml"
  if grep -q "^  name: $account$" "$scratch/runtime-existing-accounts.yaml"; then
    echo "runtime rendered an operator-owned service account" >&2
    exit 1
  fi
done
# Log access belongs only to the enabled runtime controller, with get authority.
awk '/^  name: runtime-controller$/{found=1} found && /resources: \[pods\/log\]/{getline; if ($0 ~ /verbs: \[get\]/) ok=1} /^---$/{found=0} END{exit !ok}' "$scratch/runtime.yaml"
if grep -q 'resources: \[pods/log\]' "$scratch/application.yaml"; then
  echo "runtime-disabled control plane rendered pod log authority" >&2
  exit 1
fi
grep -q 'name: registry-pull' "$scratch/runtime.yaml"
grep -q 'serviceAccountToken:' "$scratch/runtime.yaml"
grep -q 'verbs: \[create, patch, delete\]' "$scratch/runtime.yaml"
grep -q 'mountPath: /runtime-outputs' "$scratch/runtime.yaml"
grep -Fq '\"quality_repair_limit\":3' "$scratch/runtime.yaml"
helm template ayni-orchestrator "$local_chart" --set ayni-orchestrator.runtime.enabled=false \
  --namespace ayni-orchestrator-dev \
  --set-string ayni-orchestrator.images.controlPlane=local/ayni-orchestrator-control-plane:test \
  --set-string ayni-orchestrator.images.web=local/ayni-orchestrator-web:test \
  >"$scratch/local.yaml"
helm template ayni-orchestrator "$local_chart" --set ayni-orchestrator.runtime.enabled=false \
  --namespace isolated-local \
  --set ayni-orchestrator.customClients.enabled=true \
  >"$scratch/custom-clients.yaml"
helm template ayni-orchestrator "$local_chart" --set ayni-orchestrator.runtime.enabled=false \
  --namespace ayni-orchestrator-dev \
  --set observability.enabled=false \
  >"$scratch/no-observability.yaml"
helm template hot-reload "$local_chart" \
  --namespace ayni-orchestrator-dev \
  --values "$local_chart/values-hot-reload.yaml" \
  --set ayni-orchestrator.runtime.enabled=false \
  --set-string ayni-orchestrator.images.controlPlane=local/ayni-orchestrator-control-plane-dev:test \
  --set-string ayni-orchestrator.images.web=local/ayni-orchestrator-web-dev:test \
  >"$scratch/hot-reload.yaml"

# The reusable chart must render independently; runtime placement follows its namespace.
grep -Fq '\"namespace\":\"runtime-test\"' "$scratch/runtime.yaml"
grep -q 'registry.example/control-plane:test' "$scratch/application.yaml"
grep -q '^  name: control-plane$' "$scratch/application.yaml"
grep -q '^  name: web$' "$scratch/application.yaml"
if grep -Eq '^  name: (prometheus|grafana|kube-state-metrics)$' "$scratch/application.yaml"; then
  echo "reusable application chart rendered local monitoring resources" >&2
  exit 1
fi

# The local chart composes the reusable chart and version-locked upstream charts.
grep -q '# Source: ayni-orchestrator-local/charts/ayni-orchestrator/' "$scratch/local.yaml"
grep -q '# Source: ayni-orchestrator-local/charts/prometheus/' "$scratch/local.yaml"
grep -q '# Source: ayni-orchestrator-local/charts/kube-state-metrics/' "$scratch/local.yaml"
grep -q '# Source: ayni-orchestrator-local/charts/grafana/' "$scratch/local.yaml"
grep -q '^  name: prometheus$' "$scratch/local.yaml"
grep -q '^  name: grafana$' "$scratch/local.yaml"
grep -q '^  name: grafana-dashboard$' "$scratch/local.yaml"
grep -q -- '--namespaces=ayni-orchestrator-dev' "$scratch/local.yaml"
grep -q 'prom/prometheus@sha256:63805ebb8d2b3920190daf1cb14a60871b16fd38bed42b857a3182bc621f4996' "$scratch/local.yaml"
grep -q 'prometheus-config-reloader@sha256:ebb560bcb6bd3ab728a7ab61d77d22a001cc0c7a67f0531e7f92e20ce004de38' "$scratch/local.yaml"
grep -q 'grafana/grafana:12.1.1@sha256:a1701c2180249361737a99a01bc770db39381640e4d631825d38ff4535efa47d' "$scratch/local.yaml"
grep -q 'orchestrator.ayni.dev/stack: observability' "$scratch/local.yaml"
grep -q 'sizeLimit: 1Gi' "$scratch/local.yaml"
grep -q 'sizeLimit: 64Mi' "$scratch/local.yaml"
grep -q 'readOnlyRootFilesystem: false' "$scratch/hot-reload.yaml"
grep -q 'local/ayni-orchestrator-control-plane-dev:test' "$scratch/hot-reload.yaml"
grep -q 'local/ayni-orchestrator-web-dev:test' "$scratch/hot-reload.yaml"
if grep -q '@sha256:sha256:' "$scratch/local.yaml"; then
  echo "local chart rendered a malformed image digest" >&2
  exit 1
fi

grep -q '^  name: custom-client-grpc$' "$scratch/custom-clients.yaml"
grep -q 'namespace=\\"isolated-local\\"' "$scratch/custom-clients.yaml"
if grep -Eq '__AYNI_NAMESPACE__|namespace=\\"ayni-orchestrator-dev\\"' "$scratch/custom-clients.yaml"; then
  echo "isolated namespace was not propagated into the Grafana dashboard" >&2
  exit 1
fi
grep -q '^  name: ayni-orchestrator-dev-prometheus$' "$scratch/local.yaml"
grep -q -- '--wait=hookOnly' "$repository_root/dev/scripts/deploy.sh"
grep -q 'rollout status' "$repository_root/dev/scripts/deploy.sh"
grep -q 'values-hot-reload.yaml' "$repository_root/dev/scripts/deploy.sh"
grep -Fq 'get secret environment-producer --ignore-not-found -o name' "$repository_root/dev/scripts/deploy.sh"
grep -Fq 'ayni-orchestrator.connections.adminCertificates=$admin_certificates' "$repository_root/dev/scripts/deploy.sh"
helm template registered-certificate "$application_chart" --set runtime.enabled=false \
  --set-json 'connections.adminCertificates=[{"id":"certificate-admin-environment-producer","displayName":"Local environment signing","secretName":"environment-producer"}]' \
  >"$scratch/registered-certificate.yaml"
grep -Fq '\"secretName\":\"environment-producer\"' "$scratch/registered-certificate.yaml"
grep -Fq 'ORCHESTRATOR_ADMIN_CERTIFICATES: "[]"' "$scratch/application.yaml"
[[ "$(grep -c '^kind: ClusterRole$' "$scratch/local.yaml")" -eq 1 ]]
[[ "$(grep -c '^kind: ClusterRoleBinding$' "$scratch/local.yaml")" -eq 1 ]]
[[ "$(grep -c '^    helm.sh/resource-policy: keep$' "$scratch/local.yaml")" -eq 1 ]]
for claim in control-plane-state; do
  grep -q "^  name: $claim$" "$scratch/local.yaml"
done
if grep -Eq '^  name: (development-checkpoints|prepared-environments)$' "$scratch/local.yaml"; then
  echo "clean-v1 chart rendered a retired pilot volume" >&2
  exit 1
fi
if grep -Eq 'DEVELOPMENT_(CHECKPOINT|EVIDENCE|PREPARATION)_ROOT|mountPath: /(checkpoints|evidence|preparations)$' "$scratch/application.yaml"; then
  echo "clean-v1 controller rendered a retired pilot state reader" >&2
  exit 1
fi
grep -q '^  name: connection-controller$' "$scratch/application.yaml"
grep -q 'serviceAccountToken:' "$scratch/application.yaml"
if grep -Eq 'GITHUB_APP_ID|GITHUB_APP_PRIVATE_KEY_PATH|secretName: github-app-credentials' "$scratch/application.yaml"; then
  echo "GitHub credentials must be optional API references, not startup mounts" >&2
  exit 1
fi
if grep -Eq '^kind: Ingress$|^  type: (LoadBalancer|NodePort)$|nodes/proxy' "$scratch/local.yaml"; then
  echo "local chart rendered a forbidden public service or node-proxy permission" >&2
  exit 1
fi
if grep -Eq '^  name: (prometheus|grafana|kube-state-metrics)$' "$scratch/no-observability.yaml"; then
  echo "observability-disabled render still contains monitoring resources" >&2
  exit 1
fi

helm package "$application_chart" --destination "$scratch" >/dev/null
helm package "$local_chart" --destination "$scratch" >/dev/null
