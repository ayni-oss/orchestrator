#!/usr/bin/env bash
set -euo pipefail

readonly IMAGE="postgres@sha256:f3bd19c606e442c3d7bdfa8002e03fe260a1023351e0ea4598032022b68dd6e3"
readonly CONTAINER="ayni-m1-postgres-feasibility-$$"
readonly PASSWORD="feasibility-only"
readonly DATABASE="workflow_gate"

cleanup() {
  docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

if ! command -v docker >/dev/null 2>&1; then
  echo "docker is required" >&2
  exit 1
fi

docker pull "$IMAGE"
docker run --detach --rm \
  --name "$CONTAINER" \
  --env "POSTGRES_PASSWORD=$PASSWORD" \
  --env "POSTGRES_DB=$DATABASE" \
  --publish 127.0.0.1::5432 \
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

export AYNI_POSTGRES_FEASIBILITY_URL="postgresql://postgres:${PASSWORD}@127.0.0.1:${port}/${DATABASE}"
cargo test --locked -p ayni-orchestrator-workflow-postgres --test feasibility -- --ignored --nocapture
cargo test --locked -p ayni-orchestrator-workflow-postgres --test watch_feasibility -- --ignored --nocapture

echo "postgres-image=$IMAGE"
echo "postgres-platform=$(docker image inspect "$IMAGE" --format '{{.Os}}/{{.Architecture}}')"
