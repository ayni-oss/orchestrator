#!/usr/bin/env bash
# Sourced only by the disposable outbox fixture; uses its cleanup trap and helpers.
proxy_mode() { k exec nats-fault -- node -e 'require("fs").writeFileSync("/tmp/mode",process.argv[1])' "$1"; }
proxy_evidence() {
  k exec nats-fault -- cat /tmp/dropped.json >"$scratch/dropped.json"
  python3 - "$scratch/dropped.json" "$1" <<'PY'
import json,sys
assert json.load(open(sys.argv[1]))[sys.argv[2]] >= 1, 'required wire fault was not observed'
PY
}
recovery_state() {
  k cp outbox-test:/state/orchestrator.sqlite "$scratch/observed.sqlite"
  python3 "$root/dev/tests/recovery-state.py" "$scratch/orchestrator.sqlite" "$scratch/observed.sqlite" "$1"
}
restore_state() { k cp "$scratch/observed.sqlite" outbox-test:/state/orchestrator.sqlite; }
consumer_info() { nats consumer info "$stream" verification-cleanup-v1 --json >"$scratch/consumer.json"; }
assert_consumer() {
  consumer_info
  python3 - "$scratch/consumer.json" "$1" <<'PY'
import json,sys
c=json.load(open(sys.argv[1])); mode=sys.argv[2]
assert c['config']['max_ack_pending']==8 and c['config']['max_batch']==8, c
if mode=='blocked':
    assert c['num_ack_pending']==8 and c['num_pending']==2, c
elif mode=='lost':
    assert c['num_ack_pending']==1 and c['ack_floor']['stream_seq']==1, c
else:
    assert c['num_ack_pending']==0 and c['num_pending']==0, c
    assert c['ack_floor']['stream_seq']==int(mode), c
PY
}
recovery_cases() {
  local web_image=$1
  # Use the already loaded web image's Node runtime; no extra image/build required.
  k create configmap nats-fault --from-file=proxy.mjs="$root/dev/tests/nats-fault-proxy.mjs"
  cat >"$scratch/proxy.yaml" <<YAML
apiVersion: v1
kind: Service
metadata: {name: nats-fault}
spec:
  selector: {app: nats-fault}
  ports: [{port: 4222, targetPort: 4222}]
---
apiVersion: v1
kind: Pod
metadata:
  name: nats-fault
  labels: {app: nats-fault, orchestrator.ayni.dev/jetstream-client: "true"}
spec:
  restartPolicy: Never
  automountServiceAccountToken: false
  securityContext: {runAsNonRoot: true, runAsUser: 10002, runAsGroup: 10002, fsGroup: 10002, seccompProfile: {type: RuntimeDefault}}
  containers:
    - name: proxy
      image: $web_image
      imagePullPolicy: Never
      command: [node, /fixture/proxy.mjs]
      readinessProbe: {tcpSocket: {port: 4222}, periodSeconds: 1}
      resources: {requests: {cpu: 10m, memory: 32Mi}, limits: {cpu: 100m, memory: 64Mi}}
      securityContext: {allowPrivilegeEscalation: false, readOnlyRootFilesystem: true, capabilities: {drop: [ALL]}}
      volumeMounts: [{name: fixture, mountPath: /fixture, readOnly: true}, {name: scratch, mountPath: /tmp}]
  volumes:
    - name: fixture
      configMap: {name: nats-fault}
    - name: scratch
      emptyDir: {sizeLimit: 1Mi}
YAML
  k create -f "$scratch/proxy.yaml"
  k wait --for=condition=Ready pod/nats-fault --timeout=120s
  k set env deployment/control-plane ORCHESTRATOR_OUTBOX_NATS_URL=nats://nats-fault:4222
  # Suppress actual broker storage ACKs; the accepted bytes reach the broker.
  nats stream edit "$stream" --max-msgs=2 --force
  proxy_mode publish-ack
  start
  wait_metric 'orchestrator_verification_outbox_publications_total\{outcome="failed"\} [1-9]'
  stop
  proxy_evidence publisher
  inspect pending
  nats stream info "$stream" --json >"$scratch/stream.json"
  python3 - "$scratch/stream.json" <<'PY'
import json,sys
s=json.load(open(sys.argv[1]))['state']; assert s['messages']==2 and s['last_seq']==2, s
PY
  proxy_mode pass
  start
  wait_metric '^orchestrator_verification_outbox_pending 0$'
  wait_metric 'orchestrator_verification_outbox_publications_total\{outcome="duplicate"\} 1$'
  stop
  inspect stored
  # Drop the ACK for stream sequence 2 before it reaches NATS, after SQLite commit.
  # Sequence 1 is the malformed filler and is allowed to ACK normally.
  proxy_mode consumer-ack
  k set env deployment/control-plane ORCHESTRATOR_VERIFICATION_CONSUMER_ENABLED=true
  start
  wait_metric 'orchestrator_verification_consumer_deliveries_total\{outcome="woken"\} 1$'
  wait_metric 'orchestrator_verification_consumer_deliveries_total\{outcome="deferred"\} [1-9]'
  stop
  proxy_evidence consumer
  assert_consumer lost
  recovery_state lost
  restore_state
  # Keep the durable broker cursor; restart both sides with the committed receipt.
  proxy_mode pass
  k rollout restart statefulset/jetstream
  k rollout status statefulset/jetstream --timeout=120s
  start
  wait_metric 'orchestrator_verification_consumer_deliveries_total\{outcome="duplicate"\} 1$'
  wait_metric '^orchestrator_verification_consumer_unacknowledged 0$'
  assert_consumer 2
  stop
  recovery_state replay
  # Expire broker deduplication, publish exactly the original ID and bytes again.
  nats stream edit "$stream" --max-msgs=20 --dupe-window=1s --force
  sleep 2
  local id
  id=$(cat "$scratch/message-id")
  k exec outbox-test -- sh -c 'nats --server=nats://jetstream:4222 pub --jetstream orchestrator.runtime.verification.accepted.v1 --header="Nats-Msg-Id:'"$id"'" < /tmp/event.json'
  start
  wait_metric 'orchestrator_verification_consumer_deliveries_total\{outcome="duplicate"\} 1$'
  wait_metric '^orchestrator_verification_consumer_unacknowledged 0$'
  assert_consumer 3
  stop
  recovery_state replay
  # Conflicting reuse of the accepted identity is rejected once in durable storage.
  python3 - "$scratch/event.json" "$scratch/conflict.json" <<'PYCONFLICT'
import json,sys
from pathlib import Path
event=json.loads(Path(sys.argv[1]).read_text()); old=event['data']['reportSha256']
event['data']['reportSha256']='sha256:'+('a'*64 if old!='sha256:'+'a'*64 else 'b'*64)
Path(sys.argv[2]).write_text(json.dumps(event))
PYCONFLICT
  k cp "$scratch/conflict.json" outbox-test:/tmp/conflict.json
  start
  for i in {1..2}; do
    k exec outbox-test -- sh -c 'nats --server=nats://jetstream:4222 pub --jetstream orchestrator.runtime.verification.accepted.v1 < /tmp/conflict.json'
  done
  wait_metric 'orchestrator_verification_consumer_deliveries_total\{outcome="rejected"\} 2$'
  wait_metric '^orchestrator_verification_consumer_unacknowledged 0$'
  assert_consumer 5
  stop
  recovery_state conflicted
  # Fill the real receipt store with removable synthetic diagnostics (not accepted facts).
  python3 "$root/dev/tests/recovery-state.py" "$scratch/orchestrator.sqlite" "$scratch/observed.sqlite" fill
  restore_state
  # Known accepted duplicates still succeed when new receipts cannot be admitted.
  k exec outbox-test -- sh -c 'nats --server=nats://jetstream:4222 pub --jetstream orchestrator.runtime.verification.accepted.v1 < /tmp/event.json'
  for i in {1..10}; do nats pub --jetstream "$subject" "receipt-pressure-$i"; done
  start
  wait_metric 'orchestrator_verification_consumer_deliveries_total\{outcome="deferred"\} [2-9]'
  wait_metric 'orchestrator_verification_consumer_deliveries_total\{outcome="duplicate"\} 1$'
  assert_consumer blocked
  stop
  recovery_state blocked
  # Capacity release is fixture-only; production operators must use retained-history policy.
  python3 "$root/dev/tests/recovery-state.py" "$scratch/orchestrator.sqlite" "$scratch/observed.sqlite" release
  restore_state
  k rollout restart statefulset/jetstream
  k rollout status statefulset/jetstream --timeout=120s
  start
  wait_metric 'orchestrator_verification_consumer_deliveries_total\{outcome="rejected"\} 10$'
  wait_metric '^orchestrator_verification_consumer_unacknowledged 0$'
  assert_consumer 16
  stop
  recovery_state drained
  nats stream info "$stream" --json >"$scratch/stream.json"
  nats stream get "$stream" 2 --json >"$scratch/original.json"
  nats stream get "$stream" 3 --json >"$scratch/replay.json"
  python3 - "$scratch" <<'PY'
import base64,json,sys
from pathlib import Path
p=Path(sys.argv[1]); state=json.loads((p/'stream.json').read_text())['state']
assert state['messages']==16 and state['last_seq']==16, state
for file in ['original.json','replay.json']:
    m=json.loads((p/file).read_text()); m=m.get('message',m)
    assert base64.b64decode(m['data'])==(p/'event.json').read_bytes()
print('Production recovery: real ACK loss, retained durable restart, expired-window replay, stream/receipt saturation and bounded draining passed.')
PY
}
