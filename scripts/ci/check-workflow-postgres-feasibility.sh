#!/usr/bin/env bash
set -euo pipefail

readonly IMAGE="postgres@sha256:f3bd19c606e442c3d7bdfa8002e03fe260a1023351e0ea4598032022b68dd6e3"
readonly CONTAINER="ayni-m1-postgres-feasibility-$$"
readonly PASSWORD="feasibility-only"
readonly DATABASE="workflow_gate"
readonly RUNTIME_ROLE="ayni_gate_runtime"
readonly RUNTIME_PASSWORD="runtime-feasibility-only"
export M1_FIXTURE_POSTGRES_CONTAINER="$CONTAINER"

cleanup() {
  docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

if ! command -v docker >/dev/null 2>&1; then
  echo "docker is required" >&2
  exit 1
fi

docker pull "$IMAGE"
port="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')"
docker run --detach --rm \
  --name "$CONTAINER" \
  --env "POSTGRES_PASSWORD=$PASSWORD" \
  --env "POSTGRES_DB=$DATABASE" \
  --publish "127.0.0.1:${port}:5432" \
  "$IMAGE" >/dev/null

ready=false
for _ in $(seq 1 60); do
  if docker exec "$CONTAINER" pg_isready --username postgres --dbname "$DATABASE" >/dev/null 2>&1; then
    ready=true
    break
  fi
  sleep 1
done
if [[ "$ready" != true ]]; then
  echo "PostgreSQL did not become ready" >&2
  docker logs "$CONTAINER" >&2
  exit 1
fi

port="$(docker port "$CONTAINER" 5432/tcp | awk -F: 'NR == 1 {print $NF}')"
if [[ ! "$port" =~ ^[0-9]+$ ]]; then
  echo "could not determine the PostgreSQL host port" >&2
  exit 1
fi

docker exec "$CONTAINER" psql --username postgres --dbname "$DATABASE" \
  --command="CREATE ROLE $RUNTIME_ROLE LOGIN PASSWORD '$RUNTIME_PASSWORD'; \
    REVOKE ALL ON DATABASE $DATABASE FROM PUBLIC; \
    GRANT CONNECT ON DATABASE $DATABASE TO $RUNTIME_ROLE" >/dev/null

export AYNI_POSTGRES_FEASIBILITY_URL="postgresql://postgres:${PASSWORD}@127.0.0.1:${port}/${DATABASE}"
export AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL="postgresql://${RUNTIME_ROLE}:${RUNTIME_PASSWORD}@127.0.0.1:${port}/${DATABASE}"
export AYNI_POSTGRES_RUNTIME_FEASIBILITY_ROLE="$RUNTIME_ROLE"
cargo test --locked -p ayni-orchestrator-workflow-postgres --test feasibility -- --ignored --nocapture
cargo test --locked -p ayni-orchestrator-workflow-postgres --test plans -- --ignored --nocapture
# Task ownership includes concurrent claims and structured worker questions.
cargo test --locked -p ayni-orchestrator-workflow-postgres --test task_runs -- --ignored --nocapture
# Publication scenarios also validate source reclamation and exact accepted multi-upstream selections.
cargo test --locked -p ayni-orchestrator-workflow-postgres --test publications -- --ignored --nocapture
cargo test --locked -p ayni-orchestrator-workflow-postgres --test task_execution -- --ignored --nocapture
cargo test --locked -p ayni-orchestrator-workflow-postgres --test plan_approval -- --ignored --nocapture
cargo test --locked -p ayni-orchestrator-workflow-postgres --test plan_proposals -- --ignored --nocapture
# Continuity includes explicit bounded assessment consent, coalescing and restart replay.
cargo test --locked -p ayni-orchestrator-workflow-postgres --test project_continuity -- --ignored --nocapture --test-threads=1
cargo test --locked -p ayni-orchestrator-workflow-postgres --test configuration -- --ignored --nocapture
cargo test --locked -p ayni-orchestrator-workflow-postgres --test sessions -- --ignored --nocapture
cargo test --locked -p ayni-orchestrator-workflow-postgres --test provider_calls -- --ignored --nocapture
cargo test --locked -p ayni-orchestrator-workflow-postgres --test prior_schema_upgrade -- --ignored --nocapture
cargo test --locked -p ayni-orchestrator-workflow-postgres --test transport_retention -- --ignored --nocapture
export AYNI_POSTGRES_FEASIBILITY_URL="$AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL"
cargo test --locked -p ayni-orchestrator-control-plane --test workflow_api -- --nocapture
cargo test --locked -p ayni-orchestrator-control-plane --test workflow_api -- --ignored --nocapture
export AYNI_POSTGRES_FEASIBILITY_URL="postgresql://postgres:${PASSWORD}@127.0.0.1:${port}/${DATABASE}"
cargo test --locked -p ayni-orchestrator-workflow-postgres --test watch_feasibility -- --ignored --nocapture

echo "postgres-image=$IMAGE"
echo "postgres-platform=$(docker image inspect "$IMAGE" --format '{{.Os}}/{{.Architecture}}')"

if [[ "${AYNI_M1_RUN_INTEGRATED:-0}" == 1 ]]; then
  # The integrated gate keeps this disposable authority alive while the pinned
  # authenticated broker fixture exercises the full canonical gRPC handoff.
  scripts/ci/check-workflow-jetstream.sh
fi
