#!/usr/bin/env bash
# Production Helm certificate projections, watch replay, restart and lifecycle delivery.
set -euo pipefail
export HELM_NO_PLUGINS=1
[[ $# == 2 ]] || { echo 'usage: dev/tests/operator-projections.sh CONTROL_PLANE_IMAGE WEB_IMAGE' >&2; exit 2; }
root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
source "$root/dev/lib/common.sh"
namespace="ayni-operator-projections-$$"
scratch=$(mktemp -d /tmp/ayni-operator-projections.XXXXXX)
created=false
forward=''
port=${ORCHESTRATOR_PROJECTIONS_TEST_PORT:-19561}
diagnostics() {
  kubectl --context docker-desktop -n "$namespace" get pods,services,endpointslices,networkpolicies -o wide >&2 || true
  kubectl --context docker-desktop -n "$namespace" get service jetstream -o yaml >&2 || true
  kubectl --context docker-desktop -n "$namespace" get endpointslices -l kubernetes.io/service-name=jetstream -o yaml >&2 || true
  kubectl --context docker-desktop -n "$namespace" logs statefulset/jetstream --tail=100 >&2 || true
  if [[ -f "$scratch/controller.log" ]]; then cat "$scratch/controller.log" >&2; fi
  if [[ -f "$scratch/nats-error" ]]; then cat "$scratch/nats-error" >&2; fi
}
cleanup() {
  status=$?
  if [[ "$status" != 0 && "$created" == true ]]; then diagnostics; fi
  if [[ -n "$forward" ]]; then kill "$forward" 2>/dev/null || true; wait "$forward" 2>/dev/null || true; fi
  if [[ "$created" == true ]]; then
    helm uninstall operator-validation --kube-context docker-desktop -n "$namespace" --ignore-not-found >/dev/null || status=1
    kubectl --context docker-desktop delete namespace "$namespace" --wait=false >/dev/null || status=1
  fi
  rm -rf "$scratch"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
dev_build_chart_dependencies
if kubectl --context docker-desktop get namespace "$namespace" >/dev/null 2>&1; then echo 'fixture namespace already exists' >&2; exit 1; fi
kubectl --context docker-desktop create namespace "$namespace"
created=true
kubectl --context docker-desktop label namespace "$namespace" orchestrator.ayni.dev/environment=local
helm upgrade --install operator-validation "$root/dev/charts/ayni-orchestrator-local" \
  --kube-context docker-desktop -n "$namespace" --wait --timeout=5m \
  --set observability.enabled=false --set ayni-orchestrator.runtime.enabled=false \
  --set-string "ayni-orchestrator.images.controlPlane=$1" --set-string "ayni-orchestrator.images.web=$2" \
  --set ayni-orchestrator.images.pullPolicy=Never
k() { kubectl --context docker-desktop -n "$namespace" "$@"; }
start_forward() {
  k port-forward service/control-plane "$port:50051" >"$scratch/forward.log" 2>&1 &
  forward=$!
  for _ in {1..30}; do
    if grpcurl -plaintext -max-time 2 "127.0.0.1:$port" list >/dev/null 2>&1; then return; fi
    sleep 1
  done
  cat "$scratch/forward.log" >&2
  return 1
}
api() { grpcurl -plaintext -max-time 10 -d "$2" "127.0.0.1:$port" "ayni.orchestrator.v1.$1"; }
watch_capture() {
  local method=$1 payload=$2 output=$3
  # A bounded capture ends with DeadlineExceeded after receiving its snapshot/replay.
  api "$method" "$payload" >"$output" 2>"$scratch/watch-error" || {
    rg -q 'DeadlineExceeded|deadline exceeded' "$scratch/watch-error" || { cat "$scratch/watch-error" >&2; return 1; }
  }
}
start_forward
watch_capture ConnectionService/WatchConnections '{}' "$scratch/connections.json"
watch_capture ProjectService/WatchProjects '{}' "$scratch/projects.json"
for name in connections projects; do
  jq -se 'length == 1 and .[0].snapshotComplete == true' "$scratch/$name.json" >/dev/null
 done
watch_capture CertificateService/WatchCertificates '{}' "$scratch/empty-certificates.json"
jq -se 'length == 1 and .[0].snapshotComplete == true' "$scratch/empty-certificates.json" >/dev/null
api CertificateService/CreateCertificate '{"idempotencyKey":"projection-certificate-one","displayName":"Fixture one","keyId":"fixture-one","generate":true}' >"$scratch/one.json"
api CertificateService/CreateCertificate '{"idempotencyKey":"projection-certificate-two","displayName":"Fixture two","keyId":"fixture-two","generate":true}' >"$scratch/two.json"
one=$(jq -r '.certificate.id' "$scratch/one.json")
two=$(jq -r '.certificate.id' "$scratch/two.json")
watch_capture CertificateService/WatchCertificates '{}' "$scratch/snapshot.json"
jq -se '([.[]|select(.certificate != null)]|length)==2 and .[-1].snapshotComplete==true' "$scratch/snapshot.json" >/dev/null
cursor=$(jq -sr '.[-1].cursor // "0"' "$scratch/snapshot.json")
api CertificateService/SetDefaultCertificate "{\"id\":\"$two\",\"idempotencyKey\":\"projection-default\",\"expectedResourceVersion\":\"1\"}" >"$scratch/default.json"
# Invalid replacement admission cannot replace the live public key or revision.
if api CertificateService/ReplaceCertificate "{\"id\":\"$two\",\"idempotencyKey\":\"projection-rejected\",\"expectedResourceVersion\":\"2\",\"keyId\":\"bad\",\"signingSeed\":\"invalid\"}" >"$scratch/rejected.json" 2>"$scratch/rejected-error"; then
  echo 'invalid certificate replacement unexpectedly succeeded' >&2; exit 1
fi
api CertificateService/GetCertificate "{\"id\":\"$two\"}" >"$scratch/current.json"
jq -e --slurpfile original "$scratch/two.json" '.certificate.publicKey == $original[0].certificate.publicKey and .certificate.resourceVersion=="2"' "$scratch/current.json" >/dev/null
api CertificateService/DeleteCertificate "{\"id\":\"$one\",\"idempotencyKey\":\"projection-delete\",\"expectedResourceVersion\":\"2\"}" >"$scratch/deleted.json"
watch_capture CertificateService/WatchCertificates "{\"afterCursor\":\"$cursor\"}" "$scratch/replay.json"
jq -se --arg id "$one" 'any(.[]; .deleted==true and .certificate.id==$id)' "$scratch/replay.json" >/dev/null
kill "$forward" 2>/dev/null || true; wait "$forward" 2>/dev/null || true; forward=''
k rollout restart deployment/control-plane
k rollout status deployment/control-plane --timeout=180s
start_forward
api CertificateService/GetCertificate "{\"id\":\"$two\"}" >"$scratch/restarted.json"
jq -e --slurpfile prior "$scratch/current.json" '.certificate==$prior[0].certificate' "$scratch/restarted.json" >/dev/null
watch_capture CertificateService/WatchCertificates "{\"afterCursor\":\"$cursor\"}" "$scratch/restarted-replay.json"
jq -s -S . "$scratch/replay.json" >"$scratch/replay-sorted.json"
jq -s -S . "$scratch/restarted-replay.json" >"$scratch/restarted-replay-sorted.json"
cmp "$scratch/replay-sorted.json" "$scratch/restarted-replay-sorted.json"
# Give the bounded publisher time to acknowledge every accepted lifecycle fact.
sleep 7
kill "$forward" 2>/dev/null || true; wait "$forward" 2>/dev/null || true; forward=''
k logs deployment/control-plane --tail=100 >"$scratch/controller.log"
k scale deployment/control-plane --replicas=0
k wait --for=delete pod -l app.kubernetes.io/name=ayni-orchestrator-control-plane --timeout=60s
tooling=$(helm get values operator-validation --all --output=json --kube-context docker-desktop -n "$namespace" | jq -r '.jetstream.toolingImage')
cat >"$scratch/tooling.yaml" <<YAML
apiVersion: v1
kind: Pod
metadata:
  name: projection-evidence
  labels: {orchestrator.ayni.dev/jetstream-client: "true"}
spec:
  restartPolicy: Never
  automountServiceAccountToken: false
  securityContext: {runAsNonRoot: true, runAsUser: 10001, runAsGroup: 10001, fsGroup: 10001, seccompProfile: {type: RuntimeDefault}}
  containers:
  - name: tooling
    image: $tooling
    command: [sleep, "600"]
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
k wait --for=condition=Ready pod/projection-evidence --timeout=120s
k cp projection-evidence:/state/orchestrator.sqlite "$scratch/state.sqlite"
broker_ready=false
for _ in {1..10}; do
  if k exec projection-evidence -- nats --server=nats://jetstream:4222 --timeout=5s stream info ORCHESTRATOR_LIFECYCLE --json >"$scratch/stream.json" 2>"$scratch/nats-error"; then
    broker_ready=true
    break
  fi
  sleep 2
done
[[ "$broker_ready" == true ]] || { echo 'JetStream remained unreachable from its labeled fixture client' >&2; exit 1; }
last=$(jq -r '.state.last_seq' "$scratch/stream.json")
for ((seq=1;seq<=last;seq++)); do
  k exec projection-evidence -- nats --server=nats://jetstream:4222 --timeout=5s stream get ORCHESTRATOR_LIFECYCLE "$seq" --json >"$scratch/message-$seq.json"
done
python3 - "$scratch" <<'PY'
import base64,json,pathlib,sqlite3,sys
p=pathlib.Path(sys.argv[1]); c=sqlite3.connect(p/'state.sqlite')
rows=c.execute("SELECT j.id,j.resource_type,j.aggregate_revision,o.stored_at,o.payload FROM lifecycle_journal j JOIN lifecycle_outbox o ON o.journal_id=j.id WHERE o.source='ayni://orchestrator/controllers/lifecycle' ORDER BY j.rowid").fetchall()
assert len(rows)>=5, rows
assert all(r[3] is not None for r in rows), 'unacknowledged lifecycle facts'
assert any(r[1]=='certificate_deleted' for r in rows), 'deletion fact missing'
assert len({r[0] for r in rows})==len(rows), 'duplicate accepted identity'
messages=[]
delivered={}
for f in p.glob('message-*.json'):
    m=json.loads(f.read_text()); raw=m.get('data') or m.get('message',{}).get('data')
    body=base64.b64decode(raw)
    envelope=json.loads(body)
    messages.append(envelope)
    if envelope.get('source')=='ayni://orchestrator/controllers/lifecycle':
        assert envelope['id'] not in delivered, 'duplicate JetStream identity'
        delivered[envelope['id']]=body
expected={r[0]:json.loads(r[4]) for r in rows}
observed={m['id']:m for m in messages if m.get('source')=='ayni://orchestrator/controllers/lifecycle'}
assert observed==expected, 'JetStream differs from durable accepted lifecycle envelopes'
assert delivered=={r[0]:r[4] for r in rows}, 'JetStream bytes differ from immutable outbox bytes'
print(f'Operator projection/restart/replay and {len(rows)} exact lifecycle deliveries passed')
PY
