#!/usr/bin/env bash
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly FIXTURES="$ROOT/dev/feasibility/jetstream"
readonly SERVER_IMAGE="nats:2.15.0-alpine@sha256:ac8f88a6494bffc2c2a5289a0ca61cb28a9145c11ba5677cf24265d07f46d8d4"
readonly TOOL_IMAGE="natsio/nats-box:0.20.0@sha256:d2f04223f40e016f77922a8115d34b0cd5fad2ec10b19b39316ae92d07f3d295"
readonly SUFFIX="$$"
readonly SERVER="ayni-m1-js-capacity-${SUFFIX}"
readonly BOX="ayni-m1-js-capacity-box-${SUFFIX}"
readonly NETWORK="ayni-m1-js-capacity-net-${SUFFIX}"
readonly ADMIN_URL="nats://workflow-admin:admin-feasibility-only@${SERVER}:4222"
readonly APPLICATION_URL="nats://workflow-application:application-feasibility-only@${SERVER}:4222"
readonly EXECUTOR_URL="nats://workflow-executor-018f0f7b-6a4d-7a11-8d77-010203040506:executor-feasibility-only@${SERVER}:4222"
readonly SERVER_BUDGET=$((1024 * 1024 * 1024))
readonly PVC_BUDGET=$((2 * 1024 * 1024 * 1024))
WORK="$(mktemp -d /tmp/ayni-m1-jetstream-capacity.XXXXXX)"
readonly WORK

cleanup() {
  docker rm -f "$BOX" "$SERVER" >/dev/null 2>&1 || true
  docker network rm "$NETWORK" >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT INT TERM

fail() {
  echo "jetstream-capacity: $*" >&2
  exit 1
}

box_nats() {
  docker exec "$BOX" nats "$@"
}

admin() {
  box_nats --server "$ADMIN_URL" --timeout 30s "$@"
}

stream_value() {
  local stream="$1"
  local field="$2"
  admin stream info "$stream" --json | python3 -c \
    "import json,sys; print(json.load(sys.stdin)['state']['${field}'])"
}

start_server() {
  docker run --detach --name "$SERVER" --network "$NETWORK" \
    --volume "$WORK/nats.conf:/etc/nats/nats.conf:ro" \
    --volume "$WORK/data:/data" \
    "$SERVER_IMAGE" --config /etc/nats/nats.conf >/dev/null
  for _ in $(seq 1 60); do
    if docker exec "$BOX" nats --server "$ADMIN_URL" --timeout 1s \
      server check connection >/dev/null 2>&1; then
      return
    fi
    sleep 1
  done
  docker logs "$SERVER" >&2
  fail "server did not become ready"
}

publish_file() {
  local url="$1"
  local subject="$2"
  local file="$3"
  local count="$4"
  docker exec "$BOX" sh -ec \
    'cat "$1" | nats --server "$2" --timeout 30s pub --jetstream --force-stdin --count "$3" "$4"' \
    sh "$file" "$url" "$count" "$subject"
}

fill_stream() {
  local stream="$1"
  local subject="$2"
  local url="$3"
  local payload_file="$4"
  local payload_bytes="$5"
  local max_bytes="$6"
  local label="$7"
  local count=0

  docker exec "$BOX" sh -ec \
    'cat "$1" | nats --server "$2" --timeout 30s pub --jetstream --force-stdin --header "Nats-Msg-Id:$3-first" "$4"' \
    sh "$payload_file" "$url" "$label" "$subject" >/dev/null 2>&1
  count=1

  while :; do
    local used remaining conservative batch
    used="$(stream_value "$stream" bytes)"
    remaining=$((max_bytes - used))
    conservative=$((payload_bytes + 8192))
    batch=$((remaining / conservative))
    ((batch > 100)) && batch=100
    ((batch > 1)) || break
    publish_file "$url" "$subject" "$payload_file" "$batch" >/dev/null 2>&1
    count=$((count + batch))
  done

  while publish_file "$url" "$subject" "$payload_file" 1 \
    >"$WORK/${label}-last.out" 2>"$WORK/${label}-last.err"; do
    count=$((count + 1))
  done

  local used messages first_sha expected_sha remaining
  grep -Eiq 'maximum (bytes|messages) exceeded|resource limits exceeded|stream.*(full|limit)' \
    "$WORK/${label}-last.err" || {
      cat "$WORK/${label}-last.err" >&2
      fail "$stream publish failed for a reason other than its configured capacity"
    }
  used="$(stream_value "$stream" bytes)"
  messages="$(stream_value "$stream" messages)"
  remaining=$((max_bytes - used))
  ((used <= max_bytes)) || fail "$stream exceeded its configured byte bound"
  ((remaining < payload_bytes + 8192)) || fail "$stream stopped before reaching one conservative record of its byte bound"
  ((messages == count)) || fail "$stream retained $messages messages after $count successful publishes"
  ((messages > 1)) || fail "$stream did not exercise its production capacity"

  admin stream get "$stream" 1 --json >"$WORK/${label}-first.json"
  first_sha="$(python3 - "$WORK/${label}-first.json" <<'PY'
import base64
import hashlib
import json
import sys
value = json.load(open(sys.argv[1]))
data = value.get("message", value)["data"]
print(hashlib.sha256(base64.b64decode(data)).hexdigest())
PY
)"
  expected_sha="$(docker exec "$BOX" sha256sum "$payload_file" | awk '{print $1}')"
  [[ "$first_sha" == "$expected_sha" ]] || fail "$stream discarded or changed its oldest message"

  echo "capacity-stream=${stream} messages=${messages} stored-bytes=${used} configured-max-bytes=${max_bytes} discard-new=pass oldest-retained=pass"
}

profile_measurement() {
  local profile="$1"
  local server_budget="$2"
  shift 2
  local stream_sum=0 stream
  for stream in "$@"; do
    stream_sum=$((stream_sum + $(stream_value "$stream" bytes)))
  done
  local filesystem_bytes server_headroom pvc_headroom
  filesystem_bytes=$(( $(docker exec "$SERVER" du -sk /data/jetstream | awk '{print $1}') * 1024 ))
  server_headroom=$((server_budget - stream_sum))
  pvc_headroom=$((PVC_BUDGET - filesystem_bytes))
  ((server_headroom >= 512 * 1024 * 1024)) || fail "$profile leaves less than 512 MiB of logical server headroom"
  ((pvc_headroom >= 512 * 1024 * 1024)) || fail "$profile leaves less than 512 MiB of measured PVC headroom"
  echo "capacity-profile=${profile} stream-bytes=${stream_sum} filesystem-bytes=${filesystem_bytes} server-headroom=${server_headroom} pvc-headroom=${pvc_headroom}"
}

mkdir -p "$WORK/data" "$WORK/files"
chmod 0777 "$WORK/data" "$WORK/files"
cp "$FIXTURES/nats.conf" "$WORK/nats.conf"
python3 - "$WORK/files" <<'PY'
from pathlib import Path
import sys
root = Path(sys.argv[1])
for name, size, byte in (
    ("command.bin", 256 * 1024, b"C"),
    ("report.bin", 32 * 1024, b"R"),
    ("decision.bin", 16 * 1024, b"D"),
):
    (root / name).write_bytes(byte * size)
PY

docker pull "$SERVER_IMAGE" >/dev/null
docker pull "$TOOL_IMAGE" >/dev/null
docker network create "$NETWORK" >/dev/null
docker run --detach --name "$BOX" --network "$NETWORK" \
  --volume "$WORK/files:/work:ro" --volume "$FIXTURES:/config:ro" \
  "$TOOL_IMAGE" sleep infinity >/dev/null
start_server

for config in command report decision; do
  admin stream add --config "/config/${config}.json" --defaults >/dev/null
done
fill_stream ORCHESTRATOR_WORKFLOW_COMMAND_V1 \
  orchestrator.workflow.command.conversation.v1 "$APPLICATION_URL" \
  /work/command.bin $((256 * 1024)) $((256 * 1024 * 1024)) command
fill_stream ORCHESTRATOR_WORKFLOW_REPORT_V1 \
  orchestrator.workflow.report.conversation.v1 "$EXECUTOR_URL" \
  /work/report.bin $((32 * 1024)) $((192 * 1024 * 1024)) report
fill_stream ORCHESTRATOR_WORKFLOW_DECISION_V1 \
  orchestrator.workflow.accepted.conversation.v1.018f0f7b-6a4d-7a11-8d77-010203040506 "$APPLICATION_URL" \
  /work/decision.bin $((16 * 1024)) $((64 * 1024 * 1024)) decision
profile_measurement isolated "$SERVER_BUDGET" \
  ORCHESTRATOR_WORKFLOW_COMMAND_V1 ORCHESTRATOR_WORKFLOW_REPORT_V1 \
  ORCHESTRATOR_WORKFLOW_DECISION_V1

echo "jetstream-capacity: isolated-full=pass discard-new=pass filesystem-headroom=pass"
echo "nats-image=$SERVER_IMAGE"
echo "nats-box-image=$TOOL_IMAGE"
