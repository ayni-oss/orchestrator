#!/usr/bin/env bash
set -euo pipefail

readonly root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly image='nats:2.15.0-alpine@sha256:ac8f88a6494bffc2c2a5289a0ca61cb28a9145c11ba5677cf24265d07f46d8d4'
readonly container="ayni-m1-jetstream-$$"
export M1_FIXTURE_NATS_CONTAINER="$container"
readonly scratch="$(mktemp -d)"
cleanup() {
  docker rm --force "$container" >/dev/null 2>&1 || true
  rm -rf "$scratch"
}
trap cleanup EXIT INT TERM
cd "$root"

# Extract production permission/stream definitions. These are NEW deterministic
# disposable fixture passwords, never credentials read from a running stack.
python3 - "$scratch" <<'PY'
from pathlib import Path
import json, re, sys
source=Path('dev/charts/ayni-orchestrator-local/templates/jetstream.yaml').read_text()
blocks=re.findall(r'^  ([a-z-]+\.(?:json|conf)): \|\n(.*?)(?=^  [a-z-]+\.(?:json|conf): \||^\{\{- end -\}\})',source,re.M|re.S)
root=Path(sys.argv[1])
instance='018f0f7b-6a4d-7a11-8d77-010203040506'
loser='018f0f7b-6a4d-7a11-8d77-a1a2a3a4a5a6'
for name, body in blocks:
    body='\n'.join(line[4:] for line in body.splitlines())
    body=body.replace('{{ int64 .Values.jetstream.maxFileBytes }}','1073741824').replace('{{ .Values.jetstream.executorInstanceId }}',instance)
    if name=='nats.conf':
        body=body.replace('$WORKFLOW_ADMIN_PASSWORD','fixture-admin').replace('$WORKFLOW_APPLICATION_PASSWORD','fixture-application').replace('$WORKFLOW_EXECUTOR_PASSWORD','fixture-executor').replace('$WORKFLOW_ACCEPTANCE_PASSWORD','fixture-acceptance')
        start=body.index('    {\n      user: "workflow-executor"')
        end=body.index('    {\n      user: "workflow-acceptance"',start)
        losing_user=body[start:end].replace('workflow-executor','fixture-loser').replace('fixture-executor','fixture-loser').replace(instance,loser)
        body=body[:end]+losing_user+body[end:]
        root.joinpath(name).write_text(body+'\n')
    else:
        value=json.loads(body)
        # Compress only age/redelivery horizons; retain production payload, ACL,
        # byte/count caps, discard policy, file storage and durable identities.
        if name in ('command.json','report.json','decision.json'):
            value['max_age']=2_000_000_000
            value['duplicate_window']=1_000_000_000
        elif 'consumer' in name:
            value['ack_wait']=1_000_000_000
        root.joinpath(name).write_text(json.dumps(value))
        if name=='executor-decision-consumer.json':
            root.joinpath('loser-consumer.json').write_text(json.dumps(value).replace(instance,loser))
PY

docker pull "$image"
port="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')"
docker run --detach --rm --name "$container" \
  --mount "type=bind,source=$scratch,target=/fixture,readonly" \
  --publish "127.0.0.1:${port}:4222" "$image" --config /fixture/nats.conf >/dev/null
ready=false
for _ in $(seq 1 60); do
  if docker exec "$container" wget -q -O /dev/null http://127.0.0.1:8222/healthz; then ready=true; break; fi
  sleep 1
done
[[ "$ready" == true ]] || { echo 'disposable JetStream did not become ready' >&2; exit 1; }
port="$(docker port "$container" 4222/tcp | awk -F: 'NR == 1 {print $NF}')"
[[ "$port" =~ ^[0-9]+$ ]] || { echo 'invalid JetStream host port' >&2; exit 1; }
export M1_TRANSPORT_FIXTURE_DIRECTORY="$scratch"
for identity in ADMIN APPLICATION WINNER LOSER; do
  export "M1_${identity}_NATS_URL=nats://127.0.0.1:$port"
done
export M1_ADMIN_NATS_USER=workflow-admin M1_ADMIN_NATS_PASSWORD=fixture-admin
export M1_APPLICATION_NATS_USER=workflow-application M1_APPLICATION_NATS_PASSWORD=fixture-application
export M1_WINNER_NATS_USER=workflow-executor M1_WINNER_NATS_PASSWORD=fixture-executor
export M1_LOSER_NATS_USER=fixture-loser M1_LOSER_NATS_PASSWORD=fixture-loser
cargo test --locked -p ayni-orchestrator-control-plane --test workflow_transport disposable_profile_setup -- --ignored --nocapture
if [[ "${AYNI_M1_RUN_INTEGRATED:-0}" == 1 ]]; then
  : "${AYNI_POSTGRES_FEASIBILITY_URL:?integrated M1 gate requires disposable PostgreSQL}"
  : "${AYNI_POSTGRES_RUNTIME_FEASIBILITY_URL:?integrated M1 gate requires runtime PostgreSQL role}"
  : "${AYNI_POSTGRES_RUNTIME_FEASIBILITY_ROLE:?integrated M1 gate requires runtime role name}"
  cargo test --locked -p ayni-orchestrator-workflow-acceptance --test durable_handoff -- --ignored --nocapture
  # Reset only the disposable broker fixture between independent durability
  # scenarios so neither fabricated decisions nor old ACKfloors affect the next.
  cargo test --locked -p ayni-orchestrator-control-plane --test workflow_transport disposable_profile_setup -- --ignored --nocapture
fi
cargo test --locked -p ayni-orchestrator-scripted-executor authenticated_jetstream_redelivery_preserves_ledger_ownership -- --ignored --nocapture
cargo test --locked -p ayni-orchestrator-control-plane --test workflow_transport profile_authorization_capacity_and_payload -- --ignored --nocapture
printf 'jetstream-image=%s\n' "$image"
