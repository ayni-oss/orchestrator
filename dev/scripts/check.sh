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
for workload in \
  statefulset/workflow-postgres \
  statefulset/workflow-jetstream \
  statefulset/scripted-executor \
  deployment/control-plane; do
  kubectl --context "$context" --namespace "$namespace" rollout status \
    "$workload" --timeout=2m
done

postgres_pod="$(kubectl --context "$context" --namespace "$namespace" get pod \
  -l app.kubernetes.io/name=workflow-postgres \
  -o jsonpath='{.items[0].metadata.name}')"
migration_count="$(kubectl --context "$context" --namespace "$namespace" exec "$postgres_pod" -- \
  sh -ec 'psql --username="$POSTGRES_USER" --dbname="$POSTGRES_DB" --tuples-only --no-align --command="SELECT count(*) FROM workflow_schema_migrations WHERE version BETWEEN 1 AND 2"')"
latest_migration="$(kubectl --context "$context" --namespace "$namespace" exec "$postgres_pod" -- \
  sh -ec 'psql --username="$POSTGRES_USER" --dbname="$POSTGRES_DB" --tuples-only --no-align --command="SELECT max(version) FROM workflow_schema_migrations"')"
if [[ "$migration_count" != 2 || "$latest_migration" != 2 ]]; then
  echo "expected the complete Slice 2 migration history through version 2" >&2
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

jetstream_pod="$(kubectl --context "$context" --namespace "$namespace" get pod \
  -l app.kubernetes.io/name=workflow-jetstream \
  -o jsonpath='{.items[0].metadata.name}')"
jsz="${TMPDIR:-/tmp}/ayni-workflow-jsz-$.json"
trap 'rm -f "$jsz"' EXIT
kubectl --context "$context" --namespace "$namespace" exec "$jetstream_pod" -- \
  wget -q -O - 'http://127.0.0.1:8222/jsz?streams=true&consumers=true&config=true' > "$jsz"
python3 - "$jsz" <<'PY'
import json, sys
state = json.load(open(sys.argv[1]))
streams = {item["name"]: item for account in state.get("account_details", []) for item in account.get("stream_detail", [])}
expected = {
    "ORCHESTRATOR_WORKFLOW_COMMAND_V1": (268435456, 25000),
    "ORCHESTRATOR_WORKFLOW_REPORT_V1": (201326592, 50000),
    "ORCHESTRATOR_WORKFLOW_DECISION_V1": (67108864, 50000),
}
assert streams.keys() >= expected.keys(), streams.keys()
for name, (max_bytes, max_msgs) in expected.items():
    config = streams[name]["config"]
    assert config["max_bytes"] == max_bytes and config["max_msgs"] == max_msgs, (name, config)
    assert config["discard"] == "new" and config["storage"] == "file", (name, config)
    assert config["deny_delete"] and config["deny_purge"], (name, config)
consumers = {
    name: {consumer["name"] for consumer in stream.get("consumer_detail", [])}
    for name, stream in streams.items()
}
assert consumers["ORCHESTRATOR_WORKFLOW_COMMAND_V1"] == {"scripted-executor"}, consumers
assert consumers["ORCHESTRATOR_WORKFLOW_REPORT_V1"] == {"workflow-application"}, consumers
assert consumers["ORCHESTRATOR_WORKFLOW_DECISION_V1"] == {
    "executor-018f0f7b-6a4d-7a11-8d77-010203040506",
    "acceptance-harness",
}, consumers
PY

executor_pod="$(kubectl --context "$context" --namespace "$namespace" get pod \
  -l app.kubernetes.io/name=ayni-orchestrator-scripted-executor \
  -o jsonpath='{.items[0].metadata.name}')"
kubectl --context "$context" --namespace "$namespace" exec "$executor_pod" -- \
  test -f /var/lib/ayni-executor/ledger.sqlite

echo "local Slice 2 stack ready: PostgreSQL migration v2, three retained streams, control plane, and scripted executor passed"
