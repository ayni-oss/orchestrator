#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
reusable_chart="$repository_root/deploy/charts/ayni-orchestrator"
local_chart="$repository_root/dev/charts/ayni-orchestrator-local"

command -v helm >/dev/null 2>&1 || { echo "helm is required" >&2; exit 1; }
export HELM_NO_PLUGINS=1
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
export HELM_REPOSITORY_CONFIG="$scratch/repositories.yaml"
export HELM_REPOSITORY_CACHE="$scratch/cache"
export HELM_PLUGINS="$scratch/plugins"
mkdir -p "$scratch/cache" "$scratch/plugins"

helm dependency build "$local_chart" >/dev/null
helm lint "$reusable_chart" --set-string images.controlPlane=local/control-plane:test >/dev/null
helm lint "$local_chart" --set-string ayni-orchestrator.images.controlPlane=local/control-plane:test >/dev/null

reusable_render="$scratch/reusable.yaml"
local_render="$scratch/local.yaml"
helm template reusable "$reusable_chart" --namespace test \
  --set-string images.controlPlane=local/control-plane:test > "$reusable_render"
helm template local "$local_chart" --namespace test \
  --set-string ayni-orchestrator.images.controlPlane=local/control-plane:test > "$local_render"

# The reusable chart consumes administrator-owned immutable Secrets and never
# renders credentials or a database resource.
grep -q 'name: workflow-postgres-migrator' "$reusable_render"
grep -q 'name: workflow-postgres-runtime' "$reusable_render"
grep -q 'ORCHESTRATOR_WORKFLOW_MIGRATION_DATABASE_URL_PATH' "$reusable_render"
grep -q 'ORCHESTRATOR_WORKFLOW_DATABASE_URL_PATH' "$reusable_render"
grep -q 'ORCHESTRATOR_WORKFLOW_DATABASE_ROLE: "ayni_runtime"' "$reusable_render"
migration_container="$(awk '/^      initContainers:/{capture=1} /^      containers:/{if(capture){exit}} capture{print}' "$reusable_render")"
serving_container="$(awk '/^      containers:/{capture=1} /^      volumes:/{if(capture){exit}} capture{print}' "$reusable_render")"
grep -q 'name: workflow-migrate' <<<"$migration_container"
grep -q 'workflow-postgres-migrator' <<<"$migration_container"
! grep -q 'workflow-postgres-runtime' <<<"$migration_container"
grep -q 'name: control-plane' <<<"$serving_container"
grep -q 'workflow-postgres-runtime' <<<"$serving_container"
! grep -q 'workflow-postgres-migrator' <<<"$serving_container"
! grep -q '^kind: Secret$' "$reusable_render"
! grep -q 'postgresql://' "$reusable_render"
! grep -qi 'sqlite' "$reusable_render"
! grep -q 'name: web' "$reusable_render"
! grep -qi 'jetstream' "$reusable_render"

# The trusted workflow listener is container-local and absent from the Service.
grep -q 'ORCHESTRATOR_GRPC_BIND_ADDRESS: "127.0.0.1:50051"' "$reusable_render"
service_block="$(awk '/^kind: Service$/{in_service=1} /^---$/{if(in_service){exit}} in_service{print}' "$reusable_render")"
! grep -q 'trusted-grpc' <<<"$service_block"

# The local chart pins PostgreSQL, retains its claim, and wires all credentials
# through Secret references rather than ConfigMaps or command arguments.
grep -q 'kind: StatefulSet' "$local_render"
grep -q 'image: "postgres:17.6-bookworm@sha256:f3bd19c606e442c3d7bdfa8002e03fe260a1023351e0ea4598032022b68dd6e3"' "$local_render"
grep -q 'helm.sh/resource-policy: keep' "$local_render"
grep -q 'secretKeyRef:' "$local_render"
grep -q 'defaultMode: 0555' "$local_render"
grep -q "stat -c '%u:%g' /var/lib/postgresql/data" "$local_render"
grep -q 'chown -R 999:999 /var/lib/postgresql/data' "$local_render"
! grep -q 'ALTER DEFAULT PRIVILEGES' "$local_render"
! grep -q 'postgresql://' "$local_render"
! grep -qi 'sqlite' "$local_render"
! grep -qi 'jetstream' "$local_render"
! grep -q 'name: web' "$local_render"

# Schemas reject unsafe or unsupported configuration.
if helm template invalid "$reusable_chart" --set-string workflowPostgres.runtimeSecretName= >/dev/null 2>&1; then
  echo "reusable chart accepted an empty runtime Secret name" >&2
  exit 1
fi
if helm template invalid "$reusable_chart" \
  --set-string workflowPostgres.runtimeSecretName=workflow-postgres-migrator >/dev/null 2>&1; then
  echo "reusable chart accepted one Secret for migration and runtime roles" >&2
  exit 1
fi
if helm template invalid "$reusable_chart" \
  --set controlPlane.httpPort=50051 \
  --set controlPlane.trustedGrpcPort=50051 >/dev/null 2>&1; then
  echo "reusable chart accepted overlapping HTTP and trusted gRPC ports" >&2
  exit 1
fi
if helm template invalid "$local_chart" \
  --set-string postgres.secrets.runtime=workflow-postgres-migrator >/dev/null 2>&1; then
  echo "local chart accepted one Secret for migration and runtime roles" >&2
  exit 1
fi
if helm template invalid "$local_chart" --set-string postgres.image=postgres:latest >/dev/null 2>&1; then
  echo "local chart accepted an unpinned PostgreSQL image" >&2
  exit 1
fi

bash -n \
  "$repository_root/dev/ctl" \
  "$repository_root/dev/lib/common.sh" \
  "$repository_root/dev/lib/images.sh" \
  "$repository_root/dev/scripts/configure-development-secrets.sh" \
  "$repository_root/dev/scripts/deploy.sh" \
  "$repository_root/dev/scripts/check.sh"

echo "Helm PostgreSQL Slice 1 checks passed"
