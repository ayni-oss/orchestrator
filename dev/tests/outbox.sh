#!/usr/bin/env bash
# Production publisher against real JetStream, confined to one disposable namespace.
set -euo pipefail
root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
source "$root/dev/lib/common.sh"
if [[ $# != 2 ]]; then echo 'usage: dev/tests/outbox.sh CONTROL_PLANE_IMAGE WEB_IMAGE' >&2; exit 2; fi
context=$(dev_context)
consumer=false
namespace=ayni-orchestrator-outbox-test
if [[ "${AYNI_CONSUMER_FIXTURE:-false}" == true ]]; then consumer=true; namespace=ayni-orchestrator-consumer-test; fi
created_namespace=false
scratch=$(mktemp -d)
mkdir -p "$scratch/plugins"
export HELM_PLUGINS="$scratch/plugins"
forward=''
cleanup() {
  status=$?
  if [[ -n "$forward" ]]; then kill "$forward" 2>/dev/null || true; wait "$forward" 2>/dev/null || true; fi
  if [[ "$created_namespace" == true ]]; then
    helm uninstall "$DEV_RELEASE_NAME" --kube-context "$context" --namespace "$namespace" --ignore-not-found || status=1
    kubectl --context "$context" delete namespace "$namespace" --wait=false || status=1
    kubectl --context "$context" wait --for=delete "namespace/$namespace" --timeout=50s || status=1
  fi
  rm -rf "$scratch"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
dev_build_chart_dependencies
if kubectl --context "$context" get namespace "$namespace" >/dev/null 2>&1; then
  echo "Test namespace already exists: $namespace; inspect and clean it before rerunning" >&2; exit 1
fi
AYNI_OUTBOX_FIXTURE_DIRECTORY="$scratch" cargo test --locked -p ayni-orchestrator-control-plane export_outbox_fixture -- --ignored
kubectl --context "$context" create namespace "$namespace"
created_namespace=true
kubectl --context "$context" label namespace "$namespace" orchestrator.ayni.dev/environment=local
helm upgrade --install "$DEV_RELEASE_NAME" "$DEV_CHART" --kube-context "$context" --namespace "$namespace" \
  --wait --timeout=5m --set observability.enabled=false --set jetstream.lifecycle.maxMessages=1 \
  --set "ayni-orchestrator.outbox.consumerEnabled=$consumer" \
  --set ayni-orchestrator.runtime.enabled=false --set-string "ayni-orchestrator.images.controlPlane=$1" \
  --set-string "ayni-orchestrator.images.web=$2" --set ayni-orchestrator.images.pullPolicy=Never
k() { kubectl --context "$context" --namespace "$namespace" "$@"; }
stop() {
  if [[ -n "$forward" ]]; then kill "$forward" 2>/dev/null || true; wait "$forward" 2>/dev/null || true; forward=''; fi
  k scale deployment/control-plane --replicas=0
  k wait --for=delete pod -l app.kubernetes.io/name=ayni-orchestrator-control-plane --timeout=60s
}
start() {
  k scale deployment/control-plane --replicas=1
  k rollout status deployment/control-plane --timeout=120s
  k port-forward --address=127.0.0.1 service/control-plane 19080:8080 >"$scratch/forward.log" 2>&1 &
  forward=$!
  for attempt in {1..30}; do if curl -fsS --max-time 2 http://127.0.0.1:19080/readyz >/dev/null; then return; fi; sleep 1; done
  return 1
}
stop
image=$(helm get values "$DEV_RELEASE_NAME" --all --output=json --kube-context "$context" --namespace "$namespace" | python3 -c 'import json,sys; print(json.load(sys.stdin)["jetstream"]["toolingImage"])')
cat >"$scratch/tooling.yaml" <<YAML
apiVersion: v1
kind: Pod
metadata:
  name: outbox-test
  labels: {orchestrator.ayni.dev/jetstream-client: "true"}
spec:
  restartPolicy: Never
  automountServiceAccountToken: false
  securityContext: {runAsNonRoot: true, runAsUser: 10001, runAsGroup: 10001, fsGroup: 10001, seccompProfile: {type: RuntimeDefault}}
  containers:
    - name: tooling
      image: $image
      command: [sleep, "3600"]
      workingDir: /tmp
      resources: {requests: {cpu: 10m, memory: 32Mi}, limits: {cpu: 100m, memory: 64Mi}}
      securityContext: {allowPrivilegeEscalation: false, readOnlyRootFilesystem: true, capabilities: {drop: [ALL]}}
      volumeMounts: [{name: state, mountPath: /state}, {name: scratch, mountPath: /tmp}]
  volumes:
    - name: state
      persistentVolumeClaim: {claimName: control-plane-state}
    - name: scratch
      emptyDir: {sizeLimit: 16Mi}
YAML
k create -f "$scratch/tooling.yaml"
k wait --for=condition=Ready pod/outbox-test --timeout=120s
k cp "$scratch/orchestrator.sqlite" outbox-test:/state/orchestrator.sqlite
k cp "$scratch/event.json" outbox-test:/tmp/event.json
nats() { k exec outbox-test -- nats --server=nats://jetstream:4222 --timeout=5s "$@"; }
subject=orchestrator.runtime.verification.accepted.v1
stream=ORCHESTRATOR_LIFECYCLE
nats pub --jetstream "$subject" filler --header=Nats-Msg-Id:outbox-fixture-filler
wait_metric() {
  local expression=$1
  for attempt in {1..35}; do
    curl -fsS --max-time 3 http://127.0.0.1:19080/metrics >"$scratch/metrics" || true
    if grep -Eq "$expression" "$scratch/metrics"; then return; fi
    sleep 1
  done
  cat "$scratch/metrics"; return 1
}
inspect() {
  k cp outbox-test:/state/orchestrator.sqlite "$scratch/observed.sqlite"
  python3 "$root/dev/tests/inspect-outbox.py" "$scratch/orchestrator.sqlite" "$scratch/observed.sqlite" "$1"
}
# Broker outage cannot invalidate an already accepted result or application readiness.
k scale statefulset/jetstream --replicas=0
k wait --for=delete pod/jetstream-0 --timeout=60s
start
wait_metric 'orchestrator_verification_outbox_publications_total\{outcome="failed"\} [1-9]'
wait_metric '^orchestrator_verification_outbox_pending 1$'
stop
inspect pending
# The broker recovers, but a full stream must keep the exact event pending.
k scale statefulset/jetstream --replicas=1
k rollout status statefulset/jetstream --timeout=120s
start
wait_metric 'orchestrator_verification_outbox_publications_total\{outcome="failed"\} [1-9]'
stop
inspect pending
# Model the broker-store/local-ACK-write crash window: store the event while its
# durable outbox is still pending. The production publisher must get a duplicate ACK.
nats stream edit "$stream" --max-msgs=2 --force
id=$(cat "$scratch/message-id")
k exec outbox-test -- sh -c 'nats --server=nats://jetstream:4222 pub --jetstream orchestrator.runtime.verification.accepted.v1 --header="Nats-Msg-Id:'"$id"'" --header=Nats-Expected-Stream:ORCHESTRATOR_LIFECYCLE < /tmp/event.json'
start
wait_metric '^orchestrator_verification_outbox_pending 0$'
wait_metric 'orchestrator_verification_outbox_publications_total\{outcome="duplicate"\} 1$'
if [[ "$consumer" == true ]]; then
  wait_metric 'orchestrator_verification_consumer_deliveries_total\{outcome="woken"\} 1$'
  wait_metric '^orchestrator_verification_consumer_unacknowledged 0$'
fi
stop
inspect stored
if [[ "$consumer" == true ]]; then
  python3 - "$scratch/observed.sqlite" <<'PYFIXTURE'
import sqlite3,sys,json
c=sqlite3.connect(sys.argv[1]); row=c.execute("SELECT name,value FROM reconciliation_cursors WHERE name LIKE 'managed:%'").fetchone()
cursor=json.loads(row[1]); assert cursor['next_at']==0
assert c.execute("SELECT outcome,COUNT(*) FROM verification_consumer GROUP BY outcome ORDER BY outcome").fetchall()==[('rejected',1),('woken',1)]
# Later reconciliation backoff must survive application-level duplicate replay.
cursor['next_at']=9876543210
c.execute("UPDATE reconciliation_cursors SET value=? WHERE name=?",(json.dumps(cursor),row[0])); c.commit(); c.close()
PYFIXTURE
  k cp "$scratch/observed.sqlite" outbox-test:/state/orchestrator.sqlite
  # Reset only the disposable broker cursor, forcing replay independently of
  # its deduplication window and the persisted application receipt.
  nats consumer rm "$stream" verification-cleanup-v1 --force
fi
# Restarts retain the recorded ACK and bytes without a new publication.
k rollout restart statefulset/jetstream
k rollout status statefulset/jetstream --timeout=120s
start
wait_metric '^orchestrator_verification_outbox_retained_published 1$'
if [[ "$consumer" == true ]]; then
  wait_metric 'orchestrator_verification_consumer_deliveries_total\{outcome="duplicate"\} 1$'
  wait_metric '^orchestrator_verification_consumer_unacknowledged 0$'
  nats consumer info "$stream" verification-cleanup-v1 --json >"$scratch/consumer.json"
fi
stop
inspect stored
nats stream info "$stream" --json >"$scratch/info.json"
nats stream get "$stream" 2 --json >"$scratch/message.json"
python3 - "$scratch" <<'PY'
import base64,json,sys
from pathlib import Path
p=Path(sys.argv[1]); info=json.loads((p/'info.json').read_text())
assert info['state']['messages']==2 and info['state']['last_seq']==2, info
message=json.loads((p/'message.json').read_text()); message=message.get('message',message)
assert base64.b64decode(message['data'])==(p/'event.json').read_bytes()
print('Production publisher: outage, saturation, lost ACK, restart, identity and bytes passed.')
PY

if [[ "$consumer" == true ]]; then
  python3 - "$scratch" <<'PYCONSUMER'
import sqlite3,json,sys
from pathlib import Path
p=Path(sys.argv[1]); c=sqlite3.connect(p/'observed.sqlite')
cursor=json.loads(c.execute("SELECT value FROM reconciliation_cursors WHERE name LIKE 'managed:%'").fetchone()[0])
assert cursor['next_at']==9876543210, cursor
assert c.execute("SELECT outcome,COUNT(*) FROM verification_consumer GROUP BY outcome ORDER BY outcome").fetchall()==[('rejected',1),('woken',1)]
info=json.loads((p/'consumer.json').read_text())
assert info['num_ack_pending']==0 and info['num_pending']==0, info
assert info['ack_floor']['stream_seq']==2, info
before=sqlite3.connect(p/'orchestrator.sqlite')
for table in ['runtime_profiles','resource_events','verification_journal','runtime_artifacts']:
    assert before.execute(f'SELECT * FROM {table}').fetchall()==c.execute(f'SELECT * FROM {table}').fetchall(), table
print('Production consumer: committed cleanup wake, durable rejection, cursor replay, restart, unchanged projections and budgets passed.')
PYCONSUMER
fi
