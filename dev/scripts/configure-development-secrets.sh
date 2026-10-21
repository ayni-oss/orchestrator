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
    echo "$command is required to configure local workflow Secrets" >&2
    exit 1
  }
done

context="$(dev_context)"
namespace="$(dev_namespace)"
kubectl --context "$context" create namespace "$namespace" \
  --dry-run=client -o yaml | kubectl --context "$context" apply -f - >/dev/null
kubectl --context "$context" label namespace "$namespace" \
  orchestrator.ayni.dev/environment=local --overwrite >/dev/null

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
chmod 0700 "$scratch"

secret_exists() {
  kubectl --context "$context" --namespace "$namespace" get secret "$1" >/dev/null 2>&1
}

require_complete_set() {
  local label="$1"; shift
  local present=0 name
  for name in "$@"; do
    if secret_exists "$name"; then present=$((present + 1)); fi
  done
  if [[ "$present" -ne 0 && "$present" -ne "$#" ]]; then
    echo "local $label Secrets are incomplete; purge the namespace to replace the immutable set" >&2
    exit 1
  fi
  [[ "$present" -eq "$#" ]]
}

create_immutable_secret() {
  local name="$1"; shift
  kubectl --context "$context" --namespace "$namespace" create secret generic "$name" "$@" >/dev/null
  kubectl --context "$context" --namespace "$namespace" patch secret "$name" \
    --type=merge --patch '{"immutable":true}' >/dev/null
}

database=ayni_workflow
service="workflow-postgres.${namespace}.svc.cluster.local"
create_database_role_secret() {
  local name="$1" username="$2" password url directory
  directory="$scratch/$name"
  mkdir -m 0700 "$directory"
  password="$(openssl rand -hex 32)"
  url="postgresql://${username}:${password}@${service}:5432/${database}"
  printf '%s' "$username" > "$directory/username"
  printf '%s' "$password" > "$directory/password"
  printf '%s' "$url" > "$directory/url"
  create_immutable_secret "$name" \
    --from-file=username="$directory/username" \
    --from-file=password="$directory/password" \
    --from-file=url="$directory/url"
}

if require_complete_set PostgreSQL \
  workflow-postgres-admin workflow-postgres-migrator workflow-postgres-runtime; then
  echo "local PostgreSQL Secrets already exist"
else
  admin_directory="$scratch/workflow-postgres-admin"
  mkdir -m 0700 "$admin_directory"
  printf '%s' ayni_admin > "$admin_directory/username"
  openssl rand -hex 32 > "$admin_directory/password"
  printf '%s' "$database" > "$admin_directory/database"
  create_immutable_secret workflow-postgres-admin \
    --from-file=username="$admin_directory/username" \
    --from-file=password="$admin_directory/password" \
    --from-file=database="$admin_directory/database"
  create_database_role_secret workflow-postgres-migrator ayni_migrator
  create_database_role_secret workflow-postgres-runtime ayni_runtime
  echo "created immutable local PostgreSQL Secrets"
fi

create_password_secret() {
  local name="$1" directory
  directory="$scratch/$name"
  mkdir -m 0700 "$directory"
  printf '%s' "$(openssl rand -hex 32)" > "$directory/password"
  create_immutable_secret "$name" --from-file=password="$directory/password"
}

if require_complete_set JetStream \
  workflow-jetstream-admin workflow-jetstream-application \
  workflow-jetstream-executor workflow-jetstream-acceptance; then
  echo "local JetStream Secrets already exist"
else
  create_password_secret workflow-jetstream-admin
  create_password_secret workflow-jetstream-application
  create_password_secret workflow-jetstream-acceptance
  executor_directory="$scratch/workflow-jetstream-executor"
  mkdir -m 0700 "$executor_directory"
  printf '%s' "$(openssl rand -hex 32)" > "$executor_directory/password"
  printf '%s' 018f0f7b-6a4d-7a11-8d77-010203040506 > "$executor_directory/instance-id"
  printf '%s' "$(openssl rand -hex 32)" > "$executor_directory/signing-seed-hex"
  create_immutable_secret workflow-jetstream-executor \
    --from-file=password="$executor_directory/password" \
    --from-file=instance-id="$executor_directory/instance-id" \
    --from-file=signing-seed-hex="$executor_directory/signing-seed-hex"
  echo "created immutable local JetStream role Secrets"
fi

# This bearer authorizes supervisor-to-application request admission only. Provider
# API keys are separately provisioned directly in the conversation session namespace.
if secret_exists workflow-session-authority; then
  echo "local session authority Secret already exists"
else
  authority_directory="$scratch/workflow-session-authority"
  mkdir -m 0700 "$authority_directory"
  printf '%s' "$(openssl rand -hex 32)" > "$authority_directory/token"
  create_immutable_secret workflow-session-authority --from-file=token="$authority_directory/token"
  echo "created immutable local session authority Secret"
fi
