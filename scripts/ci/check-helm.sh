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
bash "$repository_root/dev/tests/check-image-build.sh"
bash "$repository_root/dev/tests/check-development-secrets.sh"
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

# Obsolete deployment-wide settings and repository-selected builder overrides fail closed.
for removed in runtime.builderDefaults.engine_image runtime.managedRuntime.build.image; do
  if helm template removed "$application_chart" --set-string "$removed=obsolete" >"$scratch/removed.yaml" 2>"$scratch/removed.err"; then
    echo "removed Helm field was accepted: $removed" >&2
    exit 1
  fi
done

helm template reusable "$application_chart" --set runtime.enabled=false \
  --namespace reusable-install \
  --set-string images.controlPlane=registry.example/control-plane:test \
  --set-string images.web=registry.example/web:test \
  >"$scratch/application.yaml"
# Existing modelSecret overrides remain inherited; explicit empty disables admin setup.
helm template provider-legacy "$application_chart" --set runtime.enabled=false \
  --set-string runtime.modelSecret=legacy-model-key >"$scratch/provider-legacy.yaml"
grep -q 'secretName: "legacy-model-key"' "$scratch/provider-legacy.yaml"
awk '/name: ORCHESTRATOR_OPENAI_SECRET_NAME/{getline; if ($0 !~ /value: "legacy-model-key"/) exit 1; found=1} END{if (!found) exit 1}' "$scratch/provider-legacy.yaml"
helm template provider-ui "$application_chart" --set runtime.enabled=false \
  --set-string runtime.modelSecret=legacy-model-key --set-string connections.openaiSecretName= \
  >"$scratch/provider-ui.yaml"
if grep -Eq 'name: (openai-api-key|OPENAI_API_KEY_PATH)$|secretName: "legacy-model-key"' "$scratch/provider-ui.yaml"; then
  echo "UI provider setup unexpectedly mounted deployment credentials" >&2
  exit 1
fi
awk '/name: ORCHESTRATOR_OPENAI_SECRET_NAME/{getline; if ($0 !~ /value: ""/) exit 1; found=1} END{if (!found) exit 1}' "$scratch/provider-ui.yaml"
helm template runtime "$application_chart" --namespace runtime-test \
  --set runtime.enabled=true \
  --set-string runtime.workerImage=registry.example/worker@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
  --set-string 'runtime.serviceAccounts.imagePullSecrets[0].name=registry-pull' \
  >"$scratch/runtime.yaml"
helm template runtime "$application_chart" --namespace runtime-test \
  --set runtime.enabled=true --set runtime.serviceAccounts.create=false \
  --set-string runtime.workerImage=registry.example/worker@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
  >"$scratch/runtime-existing-accounts.yaml"
# Namespaced observation watches cover only Pods, Jobs and Secret metadata.
for render in "$scratch/application.yaml" "$scratch/runtime.yaml"; do
  awk '/^---/{controller=0} /name: connection-controller/{controller=1} controller && /resources: \[secrets\]/{getline; if ($0 !~ /verbs: \[get, list, watch, create, patch, delete\]/) exit 1; secrets=1} controller && /resources: \[pods\]/{getline; if ($0 !~ /verbs: \[get, list, watch\]/) exit 1; pods=1} controller && /resources: \[jobs\]/{getline; if ($0 !~ /verbs: \[get, list, watch, create, delete\]/) exit 1; jobs=1} END {if (!secrets || !pods || !jobs) exit 1}' "$render"
done
# TokenReview authority exists only for opt-in verification reporting.
helm template runtime-reporting "$application_chart" --namespace runtime-test \
  --set runtime.enabled=true \
  --set-string runtime.workerImage=registry.example/worker@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
  --set-string runtime.reporting.endpoint=http://control-plane:8080/events/verification \
  --set-string runtime.reporting.source=ayni://orchestrator/supervisors/test \
  >"$scratch/runtime-reporting.yaml"
grep -q 'resources: \[tokenreviews\]' "$scratch/runtime-reporting.yaml"
grep -q 'runtime-test-runtime-reporting-verification-review' "$scratch/runtime-reporting.yaml"
if grep -q 'resources: \[tokenreviews\]' "$scratch/runtime.yaml" "$scratch/application.yaml"; then
  echo "reporting-disabled control plane rendered token review authority" >&2
  exit 1
fi
# Event authority requires transport and renders the explicit admission default.
if helm template missing-reporting "$application_chart" --set runtime.verificationMode=events >"$scratch/missing-reporting.yaml" 2>"$scratch/missing-reporting.err"; then
  echo "event verification accepted without reporting transport" >&2
  exit 1
fi
helm template event-reporting "$application_chart" --namespace runtime-test \
  --set runtime.enabled=true --set runtime.verificationMode=events \
  --set-string runtime.workerImage=registry.example/worker@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
  --set-string runtime.reporting.endpoint=http://control-plane:8080/events/verification \
  --set-string runtime.reporting.source=ayni://orchestrator/supervisors/test \
  >"$scratch/event-reporting.yaml"
grep -Fq '\"verification_mode\":\"events\"' "$scratch/event-reporting.yaml"
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
# Fixed session IPC belongs solely to the enabled control-plane runtime Role.
awk '/^---$/{role=0;controller=0} /^kind: Role$/{role=1} role && /^  name: runtime-controller$/{controller=1} /resources: \[pods\/exec\]/{if (!controller) exit 1; getline; if ($0 !~ /verbs: \[get, create\]/) exit 1; found=1} END{if (!found) exit 1}' "$scratch/runtime.yaml"
if grep -q 'resources: \[pods/exec\]' "$scratch/application.yaml"; then
  echo "runtime-disabled control plane rendered pod exec authority" >&2
  exit 1
fi
awk 'BEGIN{RS="---"} /kind: RoleBinding/ && /name: runtime-controller/{if ($0 ~ /name: runtime-(coding|builder)/ || $0 !~ /name: control-plane/) exit 1; found=1} END{if (!found) exit 1}' "$scratch/runtime.yaml"
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

helm template no-broker "$local_chart" --set ayni-orchestrator.runtime.enabled=false \
  --set jetstream.enabled=false >"$scratch/no-broker.yaml"
if grep -Eq '^  name: jetstream(-data|-config|-clients|-provision)?$|job_name: ayni-orchestrator-jetstream|jetstream.json:' "$scratch/no-broker.yaml"; then
  echo "JetStream-disabled render still contains broker resources" >&2; exit 1
fi
for invalid in jetstream.lifecycle.maxMessages=0 jetstream.serverImage=nats:latest \
  jetstream.lifecycle.maxBytes=1073741824 jetstream.maxFileBytes=0; do
  if helm template invalid "$local_chart" --set ayni-orchestrator.runtime.enabled=false \
    --set "$invalid" >"$scratch/invalid.yaml" 2>"$scratch/invalid.err"; then
    echo "invalid JetStream configuration accepted: $invalid" >&2; exit 1
  fi
done
if helm template invalid "$local_chart" --set ayni-orchestrator.runtime.enabled=false \
  --set jetstream.lifecycle.maxAgeSeconds=1 >"$scratch/invalid.yaml" 2>"$scratch/invalid.err"; then
  echo "duplicate window exceeded replay horizon" >&2; exit 1
fi
grep -q '^kind: StatefulSet$' "$scratch/local.yaml"
grep -q '^  name: jetstream-data$' "$scratch/local.yaml"
grep -q '"discard": "new"' "$scratch/local.yaml"
grep -q '"max_msg_size": 16384' "$scratch/local.yaml"
grep -q '"retention": "limits"' "$scratch/local.yaml"
grep -q '"subjects": \["orchestrator.runtime.verification.accepted.v1", "orchestrator.runtime.operation.accepted.v1", "orchestrator.coding.accepted.v1", "orchestrator.lifecycle.accepted.v1"\]' "$scratch/local.yaml"
grep -q 'job_name: ayni-orchestrator-jetstream' "$scratch/local.yaml"
grep -q 'jetstream.json:' "$scratch/local.yaml"
if grep -q 'name: exporter' "$scratch/no-observability.yaml"; then
  echo "monitoring-disabled broker rendered exporter" >&2; exit 1
fi

# The reusable chart must render independently; runtime placement follows its namespace.
grep -Fq '\"namespace\":\"runtime-test\"' "$scratch/runtime.yaml"
grep -q 'registry.example/control-plane:test' "$scratch/application.yaml"
grep -q '^  name: control-plane$' "$scratch/application.yaml"
grep -q '^  name: web$' "$scratch/application.yaml"
if grep -Eq '^  name: (prometheus|grafana|kube-state-metrics|otel-collector|jaeger|jetstream)$' "$scratch/application.yaml"; then
  echo "reusable application chart rendered local monitoring resources" >&2
  exit 1
fi

# The local chart composes the reusable chart and version-locked upstream charts.
grep -q '# Source: ayni-orchestrator-local/charts/ayni-orchestrator/' "$scratch/local.yaml"
grep -q '# Source: ayni-orchestrator-local/charts/prometheus/' "$scratch/local.yaml"
if grep -q 'name: kube-state-metrics' "$scratch/local.yaml"; then
  echo "local chart still deploys kube-state-metrics" >&2; exit 1
fi
grep -q '^  name: otel-collector$' "$scratch/local.yaml"
grep -q '^  name: jaeger$' "$scratch/local.yaml"
grep -q 'uid: jaeger' "$scratch/local.yaml"
grep -q 'OTEL_EXPORTER_OTLP_TRACES_ENDPOINT' "$scratch/local.yaml"
grep -q '# Source: ayni-orchestrator-local/charts/grafana/' "$scratch/local.yaml"
grep -q '^  name: prometheus$' "$scratch/local.yaml"
grep -q '^  name: grafana$' "$scratch/local.yaml"
grep -q '^  name: grafana-dashboard$' "$scratch/local.yaml"
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
[[ "$(grep -c '^    helm.sh/resource-policy: keep$' "$scratch/local.yaml")" -eq 2 ]]
for claim in control-plane-state jetstream-data; do
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
if grep -Eq '^  name: (prometheus|grafana|kube-state-metrics|otel-collector|jaeger|telemetry-config|tracing-config)$' "$scratch/no-observability.yaml"; then
  echo "observability-disabled render still contains monitoring resources" >&2
  exit 1
fi

helm package "$application_chart" --destination "$scratch" >/dev/null
helm package "$local_chart" --destination "$scratch" >/dev/null

# Publisher opt-in must be explicit; reusable deployments remain disabled.
if grep -q 'ORCHESTRATOR_OUTBOX_NATS_URL' "$scratch/application.yaml"; then
  echo 'Reusable chart enabled the local publisher by default' >&2; exit 1
fi
grep -q 'ORCHESTRATOR_OUTBOX_NATS_URL: "nats://jetstream:4222"' "$scratch/local.yaml"
grep -q 'orchestrator.ayni.dev/jetstream-client: "true"' "$scratch/local.yaml"
if helm template test "$application_chart" --set-string outbox.natsUrl=nats://jetstream:4222 --set outbox.localDevelopment=false >"$scratch/invalid.yaml" 2>"$scratch/invalid.err"; then
  echo 'Chart accepted publisher without trusted local opt-in' >&2; exit 1
fi
for endpoint in 'nats://user:password@jetstream:4222' 'tls://jetstream:4222' 'nats://jetstream:4222?token=fixture'; do
  if helm template test "$application_chart" --set-string "outbox.natsUrl=$endpoint" --set outbox.localDevelopment=true >"$scratch/invalid.yaml" 2>"$scratch/invalid.err"; then
    echo 'Chart accepted unsupported publisher endpoint' >&2; exit 1
  fi
done

# Application delivery defaults and required trusted-local transport.
grep -q 'ORCHESTRATOR_VERIFICATION_CONSUMER_ENABLED: "false"' "$scratch/application.yaml"
grep -q 'ORCHESTRATOR_VERIFICATION_CONSUMER_ENABLED: "true"' "$scratch/local.yaml"
if helm template invalid "$application_chart" --set outbox.consumerEnabled=true >"$scratch/invalid-consumer.yaml" 2>"$scratch/invalid-consumer.err"; then
  echo "consumer accepted without trusted local transport" >&2; exit 1
fi
