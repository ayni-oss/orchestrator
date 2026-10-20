#!/usr/bin/env bash
set -euo pipefail

readonly dev_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$dev_directory/lib/common.sh"

if [[ $# -ne 0 ]]; then
  echo "usage: dev/scripts/configure-development-secrets.sh" >&2
  exit 2
fi
for command in kubectl openssl; do
  command -v "$command" >/dev/null 2>&1 || {
    echo "$command is required to configure local PostgreSQL Secrets" >&2
    exit 1
  }
done

context="$(dev_context)"
namespace="$(dev_namespace)"
kubectl --context "$context" create namespace "$namespace" \
  --dry-run=client -o yaml | kubectl --context "$context" apply -f - >/dev/null
kubectl --context "$context" label namespace "$namespace" \
  orchestrator.ayni.dev/environment=local --overwrite >/dev/null

if kubectl --context "$context" --namespace "$namespace" get secret workflow-postgres-admin >/dev/null 2>&1; then
  for name in workflow-postgres-migrator workflow-postgres-runtime; do
    kubectl --context "$context" --namespace "$namespace" get secret "$name" >/dev/null 2>&1 || {
      echo "local PostgreSQL Secrets are incomplete; purge the namespace to replace the immutable set" >&2
      exit 1
    }
  done
  echo "local PostgreSQL Secrets already exist"
  exit 0
fi

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
chmod 0700 "$scratch"
database=ayni_workflow
service="workflow-postgres.${namespace}.svc.cluster.local"

create_role_secret() {
  local name="$1" username="$2" password url directory
  directory="$scratch/$name"
  mkdir -m 0700 "$directory"
  password="$(openssl rand -hex 32)"
  url="postgresql://${username}:${password}@${service}:5432/${database}"
  printf '%s' "$username" > "$directory/username"
  printf '%s' "$password" > "$directory/password"
  printf '%s' "$url" > "$directory/url"
  kubectl --context "$context" --namespace "$namespace" create secret generic "$name" \
    --from-file=username="$directory/username" \
    --from-file=password="$directory/password" \
    --from-file=url="$directory/url" >/dev/null
  kubectl --context "$context" --namespace "$namespace" patch secret "$name" \
    --type=merge --patch '{"immutable":true}' >/dev/null
}

admin_directory="$scratch/workflow-postgres-admin"
mkdir -m 0700 "$admin_directory"
printf '%s' ayni_admin > "$admin_directory/username"
openssl rand -hex 32 > "$admin_directory/password"
printf '%s' "$database" > "$admin_directory/database"
kubectl --context "$context" --namespace "$namespace" create secret generic workflow-postgres-admin \
  --from-file=username="$admin_directory/username" \
  --from-file=password="$admin_directory/password" \
  --from-file=database="$admin_directory/database" >/dev/null
kubectl --context "$context" --namespace "$namespace" patch secret workflow-postgres-admin \
  --type=merge --patch '{"immutable":true}' >/dev/null

create_role_secret workflow-postgres-migrator ayni_migrator
create_role_secret workflow-postgres-runtime ayni_runtime

echo "created immutable local PostgreSQL Secrets"
