#!/usr/bin/env bash
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly FIXTURES="$ROOT/dev/feasibility/jetstream"
readonly SERVER_IMAGE="nats:2.15.0-alpine@sha256:ac8f88a6494bffc2c2a5289a0ca61cb28a9145c11ba5677cf24265d07f46d8d4"
readonly TOOL_IMAGE="natsio/nats-box:0.20.0@sha256:d2f04223f40e016f77922a8115d34b0cd5fad2ec10b19b39316ae92d07f3d295"
readonly SUFFIX="$$"
readonly SERVER="ayni-m1-js-${SUFFIX}"
readonly BOX="ayni-m1-js-box-${SUFFIX}"
readonly NETWORK="ayni-m1-js-net-${SUFFIX}"
WORK="$(mktemp -d /tmp/ayni-m1-jetstream.XXXXXX)"
readonly WORK

readonly ADMIN_URL="nats://workflow-admin:admin-feasibility-only@${SERVER}:4222"
readonly APPLICATION_URL="nats://workflow-application:application-feasibility-only@${SERVER}:4222"
readonly EXECUTOR_URL="nats://workflow-executor-018f0f7b:executor-feasibility-only@${SERVER}:4222"
readonly ACCEPTANCE_URL="nats://workflow-acceptance:acceptance-feasibility-only@${SERVER}:4222"

cleanup() {
  docker rm -f "$BOX" "$SERVER" >/dev/null 2>&1 || true
  docker network rm "$NETWORK" >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT INT TERM

fail() {
  echo "jetstream-feasibility: $*" >&2
  exit 1
}

expect_failure() {
  local label="$1"
  shift
  if "$@" >"$WORK/denied.out" 2>&1; then
    cat "$WORK/denied.out" >&2
    fail "$label unexpectedly succeeded"
  fi
  echo "denied=$label"
}

expect_permission_failure() {
  local label="$1"
  shift
  if "$@" >"$WORK/denied.out" 2>&1; then
    cat "$WORK/denied.out" >&2
    fail "$label unexpectedly succeeded"
  fi
  if ! grep -Eiq 'permissions violation|authorization violation' "$WORK/denied.out"; then
    cat "$WORK/denied.out" >&2
    fail "$label failed without a permissions diagnostic"
  fi
  echo "denied=$label"
}

start_server() {
  docker run --detach --name "$SERVER" --network "$NETWORK" \
    --volume "$FIXTURES/nats.conf:/etc/nats/nats.conf:ro" \
    --volume "$WORK/data:/data" \
    "$SERVER_IMAGE" --config /etc/nats/nats.conf >/dev/null
  for _ in $(seq 1 60); do
    if docker exec "$BOX" nats --server "$ADMIN_URL" --timeout 1s server check connection >/dev/null 2>&1; then
      return
    fi
    sleep 1
  done
  docker logs "$SERVER" >&2
  fail "server did not become ready"
}

box_nats() {
  docker exec "$BOX" nats "$@"
}

admin() {
  box_nats --server "$ADMIN_URL" --timeout 3s "$@"
}

stream_messages() {
  admin stream info "$1" --json | python3 -c 'import json,sys; print(json.load(sys.stdin)["state"]["messages"])'
}

stream_bytes() {
  admin stream info "$1" --json | python3 -c 'import json,sys; print(json.load(sys.stdin)["state"]["bytes"])'
}

mkdir -p "$WORK/data" "$WORK/files"
chmod 0777 "$WORK/data" "$WORK/files"
python3 - "$WORK/files/command.bin" "$WORK/files/header.txt" <<'PY'
from pathlib import Path
import sys
Path(sys.argv[1]).write_bytes(bytes((index % 251 for index in range(256 * 1024))))
Path(sys.argv[2]).write_text("h" * 4000, encoding="ascii")
PY
readonly COMMAND_SHA="$(shasum -a 256 "$WORK/files/command.bin" | awk '{print $1}')"

docker pull "$SERVER_IMAGE" >/dev/null
docker pull "$TOOL_IMAGE" >/dev/null
docker network create "$NETWORK" >/dev/null
docker run --detach --name "$BOX" --network "$NETWORK" \
  --volume "$FIXTURES:/config:ro" --volume "$WORK/files:/work" \
  "$TOOL_IMAGE" sleep infinity >/dev/null
start_server

for config in command report decision; do
  admin stream add --config "/config/${config}.json" --defaults >/dev/null
done

# Exact rendered production bounds and isolated/coexistence headroom.
python3 - "$FIXTURES" <<'PY'
import json
import pathlib
import sys
root = pathlib.Path(sys.argv[1])
expected = {
    "command.json": ("ORCHESTRATOR_WORKFLOW_COMMAND_V1", 25_000, 256 * 1024 * 1024),
    "report.json": ("ORCHESTRATOR_WORKFLOW_REPORT_V1", 50_000, 192 * 1024 * 1024),
    "decision.json": ("ORCHESTRATOR_WORKFLOW_DECISION_V1", 50_000, 64 * 1024 * 1024),
}
total = 0
for filename, (name, messages, size) in expected.items():
    config = json.loads((root / filename).read_text())
    assert config["name"] == name
    assert config["max_msgs"] == messages
    assert config["max_bytes"] == size
    assert config["max_age"] == 7 * 24 * 60 * 60 * 1_000_000_000
    assert config["duplicate_window"] == 120 * 1_000_000_000
    assert config["max_msg_size"] == 260 * 1024
    assert config["discard"] == "new"
    assert config["storage"] == "file" and config["num_replicas"] == 1
    assert config["deny_delete"] and config["deny_purge"]
    total += size
assert total == 512 * 1024 * 1024
assert 1024 * 1024 * 1024 - total == 512 * 1024 * 1024
assert 1536 * 1024 * 1024 - total - 512 * 1024 * 1024 == 512 * 1024 * 1024
print(f"workflow-stream-bytes={total} isolated-headroom={512 * 1024 * 1024} coexistence-headroom={512 * 1024 * 1024}")
PY
for stream in ORCHESTRATOR_WORKFLOW_COMMAND_V1 ORCHESTRATOR_WORKFLOW_REPORT_V1 ORCHESTRATOR_WORKFLOW_DECISION_V1; do
  admin stream info "$stream" --json >/dev/null
done

# Anonymous access, cross-role publication, direct subscription, administration,
# deletion and purge must all fail.
expect_failure anonymous box_nats --server "nats://${SERVER}:4222" --timeout 1s server check connection
expect_failure application-publishes-report box_nats --server "$APPLICATION_URL" --timeout 1s pub --jetstream orchestrator.workflow.report.conversation.v1 denied
expect_failure executor-publishes-decision box_nats --server "$EXECUTOR_URL" --timeout 1s pub --jetstream orchestrator.workflow.accepted.conversation.v1.018f0f7b denied
expect_failure harness-publishes-command box_nats --server "$ACCEPTANCE_URL" --timeout 1s pub --jetstream orchestrator.workflow.command.conversation.v1 denied
expect_failure application-administers-streams box_nats --server "$APPLICATION_URL" --timeout 1s stream ls
expect_permission_failure executor-direct-subscribe box_nats --server "$EXECUTOR_URL" --timeout 1s sub --count 1 --wait 500ms orchestrator.workflow.command.conversation.v1
expect_failure admin-purges-command admin stream purge ORCHESTRATOR_WORKFLOW_COMMAND_V1 --force
expect_failure admin-deletes-command-message admin stream rmm ORCHESTRATOR_WORKFLOW_COMMAND_V1 1 --force

# Admin creates four durable pull consumers; workloads receive only their API and
# inbox subjects.
admin consumer add ORCHESTRATOR_WORKFLOW_COMMAND_V1 scripted-executor --pull --ack explicit --deliver all --filter orchestrator.workflow.command.conversation.v1 --max-deliver 10 --max-pending 1024 --defaults >/dev/null
admin consumer add ORCHESTRATOR_WORKFLOW_REPORT_V1 workflow-application --pull --ack explicit --deliver all --filter orchestrator.workflow.report.conversation.v1 --max-deliver 10 --max-pending 1024 --defaults >/dev/null
admin consumer add ORCHESTRATOR_WORKFLOW_DECISION_V1 executor-018f0f7b --pull --ack explicit --deliver all --filter orchestrator.workflow.accepted.conversation.v1.018f0f7b --max-deliver 10 --max-pending 1024 --defaults >/dev/null
admin consumer add ORCHESTRATOR_WORKFLOW_DECISION_V1 acceptance-harness --pull --ack explicit --deliver all --filter 'orchestrator.workflow.accepted.conversation.v1.>' --max-deliver 10 --max-pending 1024 --defaults >/dev/null

box_nats --server "$APPLICATION_URL" pub --jetstream orchestrator.workflow.command.conversation.v1 command-smoke >/dev/null
box_nats --server "$EXECUTOR_URL" consumer next ORCHESTRATOR_WORKFLOW_COMMAND_V1 scripted-executor --count 1 --ack >/dev/null
box_nats --server "$EXECUTOR_URL" pub --jetstream orchestrator.workflow.report.conversation.v1 report-smoke >/dev/null
box_nats --server "$APPLICATION_URL" consumer next ORCHESTRATOR_WORKFLOW_REPORT_V1 workflow-application --count 1 --ack >/dev/null
box_nats --server "$APPLICATION_URL" pub --jetstream orchestrator.workflow.accepted.conversation.v1.018f0f7b decision-smoke >/dev/null
box_nats --server "$EXECUTOR_URL" consumer next ORCHESTRATOR_WORKFLOW_DECISION_V1 executor-018f0f7b --count 1 --ack >/dev/null
box_nats --server "$ACCEPTANCE_URL" consumer next ORCHESTRATOR_WORKFLOW_DECISION_V1 acceptance-harness --count 1 --ack >/dev/null

# Publish the exact maximum command body with a near-4-KiB application header,
# measure broker-reported storage overhead, and retain its digest across restart.
before_bytes="$(stream_bytes ORCHESTRATOR_WORKFLOW_COMMAND_V1)"
docker exec "$BOX" sh -ec 'cat /work/command.bin | nats --server "$1" --timeout 5s pub --jetstream --force-stdin --header "Nats-Msg-Id:command-max-fixture" --header "X-Ayni-Fixture:$(cat /work/header.txt)" orchestrator.workflow.command.conversation.v1' sh "$APPLICATION_URL" >/dev/null
after_bytes="$(stream_bytes ORCHESTRATOR_WORKFLOW_COMMAND_V1)"
delta=$((after_bytes - before_bytes))
(( delta > 262144 )) || fail "broker storage delta did not include metadata"
(( delta <= 270336 )) || fail "8-KiB storage margin is insufficient: delta=$delta"
echo "max-command-body=262144 broker-stored-delta=$delta margin=$((delta - 262144))"
messages_before_restart="$(stream_messages ORCHESTRATOR_WORKFLOW_COMMAND_V1)"

docker stop "$SERVER" >/dev/null
docker rm "$SERVER" >/dev/null
start_server
republish_output="$(docker exec "$BOX" sh -ec 'cat /work/command.bin | nats --server "$1" --timeout 5s pub --jetstream --force-stdin --header "Nats-Msg-Id:command-max-fixture" --header "X-Ayni-Fixture:$(cat /work/header.txt)" orchestrator.workflow.command.conversation.v1' sh "$APPLICATION_URL" 2>&1)"
if ! grep -Fq 'Sequence: 2 Duplicate: true' <<<"$republish_output"; then
  printf '%s\n' "$republish_output" >&2
  fail "republish did not return the original sequence with duplicate=true"
fi
messages_after_republish="$(stream_messages ORCHESTRATOR_WORKFLOW_COMMAND_V1)"
[[ "$messages_after_republish" == "$messages_before_restart" ]] || fail "stable message ID did not deduplicate after restart"

admin stream get ORCHESTRATOR_WORKFLOW_COMMAND_V1 2 --json > "$WORK/files/stored.json"
stored_sha="$(python3 - "$WORK/files/stored.json" <<'PY'
import base64
import hashlib
import json
import sys
value = json.load(open(sys.argv[1]))
data = value.get("message", value).get("data")
print(hashlib.sha256(base64.b64decode(data)).hexdigest())
PY
)"
[[ "$stored_sha" == "$COMMAND_SHA" ]] || fail "stored command bytes changed"
echo "exact-republish-sha256=$stored_sha"

# Scaled fixtures exercise the same discard-new and age-expiry mechanisms without
# writing the full 512-MiB production reservation during every CI run.
admin stream add M1_CAPACITY_FIXTURE --subjects fixture.capacity --storage file --retention limits --discard new --max-msgs 1 --max-bytes 1MiB --max-msg-size 16KiB --replicas 1 --defaults >/dev/null
admin pub --jetstream fixture.capacity oldest >/dev/null
expect_failure discard-new-full admin pub --jetstream fixture.capacity rejected
[[ "$(stream_messages M1_CAPACITY_FIXTURE)" == 1 ]] || fail "discard-new replaced the retained oldest record"
admin stream add M1_EXPIRY_FIXTURE --subjects fixture.expiry --storage file --retention limits --discard new --max-msgs 10 --max-bytes 1MiB --max-msg-size 16KiB --max-age 500ms --replicas 1 --defaults >/dev/null
admin pub --jetstream fixture.expiry expires >/dev/null
sleep 2
[[ "$(stream_messages M1_EXPIRY_FIXTURE)" == 0 ]] || fail "age expiry did not remove the fixture record"

echo "jetstream-feasibility: auth=pass acls=pass consumers=pass bounds=pass payload=pass overhead=pass restart-republish=pass discard-new=pass expiry=pass"
echo "nats-image=$SERVER_IMAGE"
echo "nats-box-image=$TOOL_IMAGE"
