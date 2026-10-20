#!/usr/bin/env bash
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly FIXTURES="$ROOT/dev/feasibility/jetstream"
readonly CONTEXT="docker-desktop"
readonly NAMESPACE="ayni-m1-js-credentials-$$"
readonly SERVER_IMAGE="nats:2.15.0-alpine@sha256:ac8f88a6494bffc2c2a5289a0ca61cb28a9145c11ba5677cf24265d07f46d8d4"
readonly TOOL_IMAGE="natsio/nats-box:0.20.0@sha256:d2f04223f40e016f77922a8115d34b0cd5fad2ec10b19b39316ae92d07f3d295"
WORK="$(mktemp -d /tmp/ayni-m1-jetstream-credentials.XXXXXX)"
readonly WORK
created=false

k() {
  kubectl --context "$CONTEXT" --namespace "$NAMESPACE" "$@"
}

cleanup() {
  local status=$?
  if [[ "$status" != 0 && "$created" == true ]]; then
    k get pods -o wide >&2 || true
    k logs pod/nats --tail=100 >&2 || true
  fi
  if [[ "$created" == true ]]; then
    kubectl --context "$CONTEXT" delete namespace "$NAMESPACE" --wait=false >/dev/null 2>&1 || true
    kubectl --context "$CONTEXT" wait --for=delete "namespace/$NAMESPACE" --timeout=90s >/dev/null 2>&1 || true
  fi
  rm -rf "$WORK"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

fail() {
  echo "jetstream-credentials: $*" >&2
  exit 1
}

credential_url() {
  case "$1" in
    admin) echo 'nats://workflow-admin:admin-feasibility-only@nats:4222' ;;
    application) echo 'nats://workflow-application:application-feasibility-only@nats:4222' ;;
    executor-winner) echo 'nats://workflow-executor-018f0f7b-6a4d-7a11-8d77-010203040506:executor-feasibility-only@nats:4222' ;;
    executor-loser) echo 'nats://workflow-executor-018f0f7b-6a4d-7a11-8d77-a1a2a3a4a5a6:executor-a1-feasibility-only@nats:4222' ;;
    acceptance) echo 'nats://workflow-acceptance:acceptance-feasibility-only@nats:4222' ;;
    *) fail "unknown fixture identity: $1" ;;
  esac
}

create_identity() {
  local identity="$1"
  k create serviceaccount "$identity" >/dev/null
  k create secret generic "${identity}-credentials" \
    --from-literal=url="$(credential_url "$identity")" >/dev/null
  k patch secret "${identity}-credentials" --type=merge -p '{"immutable":true}' >/dev/null
}

create_client() {
  local identity="$1"
  cat >"$WORK/${identity}.yaml" <<YAML
apiVersion: v1
kind: Pod
metadata:
  name: ${identity}
  labels:
    orchestrator.ayni.dev/workflow-identity: ${identity}
spec:
  restartPolicy: Never
  automountServiceAccountToken: false
  serviceAccountName: ${identity}
  securityContext:
    runAsNonRoot: true
    runAsUser: 10001
    runAsGroup: 10001
    fsGroup: 10001
    seccompProfile: {type: RuntimeDefault}
  containers:
    - name: nats
      image: ${TOOL_IMAGE}
      workingDir: /tmp
      command: [sleep, infinity]
      resources:
        requests: {cpu: 10m, memory: 32Mi}
        limits: {cpu: 100m, memory: 64Mi}
      securityContext:
        allowPrivilegeEscalation: false
        readOnlyRootFilesystem: true
        capabilities: {drop: [ALL]}
      volumeMounts:
        - {name: credentials, mountPath: /credentials, readOnly: true}
        - {name: stream-config, mountPath: /streams, readOnly: true}
        - {name: scratch, mountPath: /tmp}
  volumes:
    - name: credentials
      secret: {secretName: ${identity}-credentials}
    - name: stream-config
      configMap: {name: stream-config}
    - name: scratch
      emptyDir: {sizeLimit: 8Mi}
YAML
  k create -f "$WORK/${identity}.yaml" >/dev/null
}

nats_as() {
  local identity="$1"
  shift
  k exec "$identity" -- sh -ec \
    'url=$(cat /credentials/url); exec nats --server "$url" --timeout 5s "$@"' sh "$@"
}

admin() {
  nats_as admin "$@"
}

admin_stream_add() {
  local config="$1"
  admin stream add --config "/streams/$(basename "$config")" --defaults
}

expect_failure() {
  local label="$1"
  shift
  if "$@" >"$WORK/${label}.out" 2>&1; then
    cat "$WORK/${label}.out" >&2
    fail "$label unexpectedly succeeded"
  fi
  echo "denied=$label"
}

ack_pending() {
  admin consumer info ORCHESTRATOR_WORKFLOW_DECISION_V1 executor-018f0f7b-6a4d-7a11-8d77-a1a2a3a4a5a6 --json |
    python3 -c 'import json,sys; print(json.load(sys.stdin)["num_ack_pending"])'
}

kubectl --context "$CONTEXT" get nodes >/dev/null
if kubectl --context "$CONTEXT" get namespace "$NAMESPACE" >/dev/null 2>&1; then
  fail "fixture namespace already exists"
fi
kubectl --context "$CONTEXT" create namespace "$NAMESPACE" >/dev/null
created=true
k label namespace "$NAMESPACE" orchestrator.ayni.dev/environment=disposable >/dev/null

if grep -Eq '(admin|application|executor|executor-a1|acceptance)-feasibility-only' \
  "$FIXTURES/nats.conf"; then
  fail "broker ConfigMap contains plaintext client credentials"
fi
k create configmap nats-config --from-file=nats.conf="$FIXTURES/nats.conf" >/dev/null
k create configmap stream-config \
  --from-file=command.json="$FIXTURES/command.json" \
  --from-file=report.json="$FIXTURES/report.json" \
  --from-file=decision.json="$FIXTURES/decision.json" >/dev/null
for identity in admin application executor-winner executor-loser acceptance; do
  create_identity "$identity"
done
k create serviceaccount diagnostic >/dev/null

cat >"$WORK/nats.yaml" <<YAML
apiVersion: v1
kind: Pod
metadata: {name: nats, labels: {app: nats}}
spec:
  restartPolicy: Never
  automountServiceAccountToken: false
  securityContext:
    runAsNonRoot: true
    runAsUser: 10001
    runAsGroup: 10001
    fsGroup: 10001
    seccompProfile: {type: RuntimeDefault}
  containers:
    - name: nats
      image: ${SERVER_IMAGE}
      args: [--config=/config/nats.conf]
      ports: [{containerPort: 4222, name: client}]
      readinessProbe: {httpGet: {path: "/healthz?js-enabled-only=true", port: 8222}, periodSeconds: 1}
      resources:
        requests: {cpu: 25m, memory: 64Mi}
        limits: {cpu: 500m, memory: 256Mi}
      securityContext:
        allowPrivilegeEscalation: false
        readOnlyRootFilesystem: true
        capabilities: {drop: [ALL]}
      volumeMounts:
        - {name: config, mountPath: /config, readOnly: true}
        - {name: data, mountPath: /data}
  volumes:
    - name: config
      configMap: {name: nats-config}
    - name: data
      emptyDir: {sizeLimit: 1536Mi}
---
apiVersion: v1
kind: Service
metadata: {name: nats}
spec:
  selector: {app: nats}
  ports: [{name: client, port: 4222, targetPort: client}]
YAML
k create -f "$WORK/nats.yaml" >/dev/null
k wait --for=condition=Ready pod/nats --timeout=120s >/dev/null

for identity in admin application executor-winner executor-loser acceptance; do
  create_client "$identity"
done
cat >"$WORK/diagnostic.yaml" <<YAML
apiVersion: v1
kind: Pod
metadata: {name: diagnostic}
spec:
  restartPolicy: Never
  automountServiceAccountToken: false
  serviceAccountName: diagnostic
  securityContext: {runAsNonRoot: true, runAsUser: 10001, runAsGroup: 10001, seccompProfile: {type: RuntimeDefault}}
  containers:
    - name: nats
      image: ${TOOL_IMAGE}
      workingDir: /tmp
      command: [sleep, infinity]
      resources: {requests: {cpu: 10m, memory: 32Mi}, limits: {cpu: 100m, memory: 64Mi}}
      securityContext: {allowPrivilegeEscalation: false, readOnlyRootFilesystem: true, capabilities: {drop: [ALL]}}
YAML
k create -f "$WORK/diagnostic.yaml" >/dev/null
k wait --for=condition=Ready pod --all --timeout=120s >/dev/null

for identity in admin application executor-winner executor-loser acceptance; do
  k get pod "$identity" -o json >"$WORK/${identity}-pod.json"
  python3 - "$identity" "$WORK/${identity}-pod.json" <<'PY'
import json
import sys
identity = sys.argv[1]
pod = json.load(open(sys.argv[2]))
assert pod["spec"].get("automountServiceAccountToken") is False
secrets = [v["secret"]["secretName"] for v in pod["spec"]["volumes"] if "secret" in v]
assert secrets == [f"{identity}-credentials"], secrets
assert all("envFrom" not in c for c in pod["spec"]["containers"])
PY
  if kubectl --context "$CONTEXT" auth can-i get secrets \
    --as="system:serviceaccount:${NAMESPACE}:${identity}" --namespace "$NAMESPACE" --quiet; then
    fail "$identity service account can read Kubernetes Secrets"
  fi
done
if ! k get pod diagnostic -o json | python3 -c \
  'import json,sys; p=json.load(sys.stdin); assert not any("secret" in v for v in p["spec"].get("volumes", []))'; then
  fail "diagnostic pod received a credential Secret"
fi
expect_failure diagnostic-anonymous k exec diagnostic -- nats --server nats://nats:4222 --timeout 2s server check connection

for config in command report decision; do
  admin_stream_add "$FIXTURES/${config}.json" >/dev/null
done
admin consumer add ORCHESTRATOR_WORKFLOW_COMMAND_V1 scripted-executor \
  --pull --ack explicit --wait 1s --deliver all \
  --filter orchestrator.workflow.command.conversation.v1 --max-deliver 10 \
  --max-pending 16 --defaults >/dev/null
admin consumer add ORCHESTRATOR_WORKFLOW_DECISION_V1 executor-018f0f7b-6a4d-7a11-8d77-010203040506 \
  --pull --ack explicit --wait 1s --deliver all \
  --filter orchestrator.workflow.accepted.conversation.v1.018f0f7b-6a4d-7a11-8d77-010203040506 \
  --max-deliver 10 --max-pending 16 --defaults >/dev/null
admin consumer add ORCHESTRATOR_WORKFLOW_DECISION_V1 executor-018f0f7b-6a4d-7a11-8d77-a1a2a3a4a5a6 \
  --pull --ack explicit --wait 1s --deliver all \
  --filter orchestrator.workflow.accepted.conversation.v1.018f0f7b-6a4d-7a11-8d77-a1a2a3a4a5a6 \
  --max-deliver 10 --max-pending 16 --defaults >/dev/null
admin consumer add ORCHESTRATOR_WORKFLOW_DECISION_V1 acceptance-harness \
  --pull --ack explicit --wait 1s --deliver all \
  --filter 'orchestrator.workflow.accepted.conversation.v1.>' \
  --max-deliver 10 --max-pending 16 --defaults >/dev/null

nats_as application pub --jetstream orchestrator.workflow.command.conversation.v1 command-fixture >/dev/null
nats_as executor-winner consumer next ORCHESTRATOR_WORKFLOW_COMMAND_V1 scripted-executor --count 1 --ack >/dev/null
expect_failure winner-reads-loser-decision nats_as executor-winner consumer info ORCHESTRATOR_WORKFLOW_DECISION_V1 executor-018f0f7b-6a4d-7a11-8d77-a1a2a3a4a5a6
expect_failure loser-reads-winner-decision nats_as executor-loser consumer info ORCHESTRATOR_WORKFLOW_DECISION_V1 executor-018f0f7b-6a4d-7a11-8d77-010203040506
expect_failure acceptance-publishes nats_as acceptance pub --jetstream orchestrator.workflow.command.conversation.v1 denied

nats_as application pub --jetstream orchestrator.workflow.accepted.conversation.v1.018f0f7b-6a4d-7a11-8d77-a1a2a3a4a5a6 decision-fixture >/dev/null
nats_as executor-loser consumer next ORCHESTRATOR_WORKFLOW_DECISION_V1 executor-018f0f7b-6a4d-7a11-8d77-a1a2a3a4a5a6 --count 1 --no-ack >/dev/null
[[ "$(ack_pending)" == 1 ]] || fail "unacknowledged targeted decision was not retained"
echo 'retirement-blocked=targeted-decision-unacked'
k get secret executor-loser-credentials >/dev/null
admin consumer info ORCHESTRATOR_WORKFLOW_DECISION_V1 executor-018f0f7b-6a4d-7a11-8d77-a1a2a3a4a5a6 >/dev/null

sleep 2
nats_as executor-loser consumer next ORCHESTRATOR_WORKFLOW_DECISION_V1 executor-018f0f7b-6a4d-7a11-8d77-a1a2a3a4a5a6 --count 1 --ack >/dev/null
[[ "$(ack_pending)" == 0 ]] || fail "targeted decision ACK did not clear retirement blocker"
admin consumer rm ORCHESTRATOR_WORKFLOW_DECISION_V1 executor-018f0f7b-6a4d-7a11-8d77-a1a2a3a4a5a6 --force >/dev/null
k delete pod executor-loser --wait=true >/dev/null
k delete secret executor-loser-credentials >/dev/null
expect_failure retired-consumer admin consumer info ORCHESTRATOR_WORKFLOW_DECISION_V1 executor-018f0f7b-6a4d-7a11-8d77-a1a2a3a4a5a6
if k get secret executor-loser-credentials >/dev/null 2>&1; then
  fail "retired executor credential Secret still exists"
fi

echo 'jetstream-credentials: immutable-secrets=pass broker-hashes=pass pod-isolation=pass rbac-denial=pass executor-isolation=pass retirement-order=pass'
echo "nats-image=$SERVER_IMAGE"
echo "nats-box-image=$TOOL_IMAGE"
