#!/usr/bin/env bash
set -euo pipefail

readonly dev_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$dev_directory/lib/common.sh"

expect_empty=false
if [[ $# -eq 1 && "$1" == --expect-empty ]]; then
  expect_empty=true
elif [[ $# -ne 0 ]]; then
  echo "usage: dev/ctl check [--expect-empty]" >&2
  exit 2
fi

context="$(dev_context)"
namespace="$(dev_namespace)"
kubectl --context "$context" --namespace "$namespace" rollout status \
  statefulset/workflow-postgres --timeout=2m
kubectl --context "$context" --namespace "$namespace" rollout status \
  deployment/control-plane --timeout=2m

postgres_pod="$(kubectl --context "$context" --namespace "$namespace" get pod \
  -l app.kubernetes.io/name=workflow-postgres \
  -o jsonpath='{.items[0].metadata.name}')"
migration_count="$(kubectl --context "$context" --namespace "$namespace" exec "$postgres_pod" -- \
  sh -ec 'psql --username="$POSTGRES_USER" --dbname="$POSTGRES_DB" --tuples-only --no-align --command="SELECT count(*) FROM workflow_schema_migrations WHERE version = 1"')"
if [[ "$migration_count" != 1 ]]; then
  echo "expected exactly one applied Slice 1 schema migration" >&2
  exit 1
fi

if [[ "$expect_empty" == true ]]; then
  workspace_count="$(kubectl --context "$context" --namespace "$namespace" exec "$postgres_pod" -- \
    sh -ec 'psql --username="$POSTGRES_USER" --dbname="$POSTGRES_DB" --tuples-only --no-align --command="SELECT count(*) FROM workflow_workspaces"')"
  if [[ "$workspace_count" != 0 ]]; then
    echo "expected a fresh workflow database, found $workspace_count workspaces" >&2
    exit 1
  fi
fi

echo "local stack ready: PostgreSQL 17.6 authority, migration v1, and control-plane readiness passed"
