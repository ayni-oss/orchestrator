#!/usr/bin/env bash
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly FIXTURES="$ROOT/dev/feasibility/jetstream"
readonly SERVER_IMAGE="nats:2.15.0-alpine@sha256:ac8f88a6494bffc2c2a5289a0ca61cb28a9145c11ba5677cf24265d07f46d8d4"
readonly TOOL_IMAGE="natsio/nats-box:0.20.0@sha256:d2f04223f40e016f77922a8115d34b0cd5fad2ec10b19b39316ae92d07f3d295"
readonly SUFFIX="$$"
readonly SERVER="ayni-m1-executor-${SUFFIX}"
readonly BOX="ayni-m1-executor-box-${SUFFIX}"
readonly NETWORK="ayni-m1-executor-net-${SUFFIX}"
readonly WINNER="018f0f7b-6a4d-7a11-8d77-010203040506"
readonly LOSER="018f0f7b-6a4d-7a11-8d77-a1a2a3a4a5a6"
WORK="$(mktemp -d /tmp/ayni-m1-executor.XXXXXX)"
readonly WORK

cleanup() {
  docker rm -f "$BOX" "$SERVER" >/dev/null 2>&1 || true
  docker network rm "$NETWORK" >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT INT TERM

fail() {
  echo "executor-feasibility: $*" >&2
  exit 1
}

box_nats() {
  docker exec "$BOX" nats "$@"
}

admin() {
  box_nats --server "nats://workflow-admin:admin-feasibility-only@${SERVER}:4222" \
    --timeout 5s "$@"
}

mkdir -p "$WORK/data" "$WORK/config"
chmod 0777 "$WORK/data"
cp "$FIXTURES/command.json" "$FIXTURES/decision.json" "$WORK/config/"
python3 - "$FIXTURES/report.json" "$WORK/config/report.json" <<'PY'
import json
import sys
value = json.load(open(sys.argv[1]))
# Short horizons represent broker age expiry and the end of deduplication without
# making the gate wait seven days. Exact bytes remain owned by the executor ledger.
value["max_age"] = 2_000_000_000
value["duplicate_window"] = 1_000_000_000
with open(sys.argv[2], "w") as output:
    json.dump(value, output, separators=(",", ":"))
PY

docker pull "$SERVER_IMAGE" >/dev/null
docker pull "$TOOL_IMAGE" >/dev/null
docker network create "$NETWORK" >/dev/null
docker run --detach --name "$BOX" --network "$NETWORK" \
  --volume "$WORK/config:/config:ro" "$TOOL_IMAGE" sleep infinity >/dev/null
docker run --detach --name "$SERVER" --network "$NETWORK" \
  --publish 127.0.0.1::4222 \
  --volume "$FIXTURES/nats.conf:/etc/nats/nats.conf:ro" \
  --volume "$WORK/data:/data" \
  "$SERVER_IMAGE" --config /etc/nats/nats.conf >/dev/null
for _ in $(seq 1 60); do
  if admin server check connection >/dev/null 2>&1; then break; fi
  sleep 1
done
admin server check connection >/dev/null || {
  docker logs "$SERVER" >&2
  fail "server did not become ready"
}

for config in command report decision; do
  admin stream add --config "/config/${config}.json" --defaults >/dev/null
done
admin consumer add ORCHESTRATOR_WORKFLOW_COMMAND_V1 scripted-executor \
  --pull --ack explicit --wait 1s --deliver all \
  --filter orchestrator.workflow.command.conversation.v1 --max-deliver 10 \
  --max-pending 16 --defaults >/dev/null
admin consumer add ORCHESTRATOR_WORKFLOW_REPORT_V1 workflow-application \
  --pull --ack explicit --wait 1s --deliver all \
  --filter orchestrator.workflow.report.conversation.v1 --max-deliver 10 \
  --max-pending 16 --defaults >/dev/null
admin consumer add ORCHESTRATOR_WORKFLOW_DECISION_V1 "executor-${WINNER}" \
  --pull --ack explicit --wait 1s --deliver all \
  --filter "orchestrator.workflow.accepted.conversation.v1.${WINNER}" \
  --max-deliver 10 --max-pending 16 --defaults >/dev/null
admin consumer add ORCHESTRATOR_WORKFLOW_DECISION_V1 "executor-${LOSER}" \
  --pull --ack explicit --wait 1s --deliver all \
  --filter "orchestrator.workflow.accepted.conversation.v1.${LOSER}" \
  --max-deliver 10 --max-pending 16 --defaults >/dev/null

host_address="$(docker port "$SERVER" 4222/tcp | head -1)"
host_port="${host_address##*:}"
[[ "$host_port" =~ ^[0-9]+$ ]] || fail "could not resolve the loopback fixture port"
readonly HOST_URL="nats://127.0.0.1:${host_port}"
export M1_ADMIN_NATS_URL="$HOST_URL"
export M1_ADMIN_NATS_USER="workflow-admin"
export M1_ADMIN_NATS_PASSWORD="admin-feasibility-only"
export M1_APPLICATION_NATS_URL="$HOST_URL"
export M1_APPLICATION_NATS_USER="workflow-application"
export M1_APPLICATION_NATS_PASSWORD="application-feasibility-only"
export M1_WINNER_NATS_URL="$HOST_URL"
export M1_WINNER_NATS_USER="workflow-executor-${WINNER}"
export M1_WINNER_NATS_PASSWORD="executor-feasibility-only"
export M1_LOSER_NATS_URL="$HOST_URL"
export M1_LOSER_NATS_USER="workflow-executor-${LOSER}"
export M1_LOSER_NATS_PASSWORD="executor-a1-feasibility-only"

cargo test --locked --manifest-path "$ROOT/Cargo.toml" \
  -p ayni-orchestrator-scripted-executor \
  authenticated_jetstream_redelivery_preserves_ledger_ownership \
  -- --ignored --nocapture

echo 'executor-feasibility: authenticated-transport=pass crash-redelivery=pass split-brain=pass ack-reconciliation=pass long-outage=pass retirement=pass'
echo "nats-image=$SERVER_IMAGE"
echo "nats-box-image=$TOOL_IMAGE"
