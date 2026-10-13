#!/usr/bin/env bash
# Genuine coding supervisor, Pod-bound receipt, early acceptance and restart.
# Invalid public runtime metadata produces a truthful AdmissionFailed result;
# this fixture does not claim managed-runtime or provider compatibility.
set -euo pipefail
export HELM_NO_PLUGINS=1
[[ $# == 3 ]] || { echo 'usage: dev/tests/coding-reporting.sh CONTROL_PLANE_IMAGE PINNED_WORKER_IMAGE WEB_IMAGE' >&2; exit 2; }
root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
namespace=ayni-orchestrator-coding-test
scratch=$(mktemp -d)
created=false
mode=${AYNI_CODING_DELIVERY:-live}
case "$mode" in
  live) supervisor_delay=240 ;;
  retained) supervisor_delay=0 ;;
  *) echo 'AYNI_CODING_DELIVERY must be live or retained' >&2; exit 2 ;;
esac
cleanup() {
  status=$?
  if [[ "$status" != 0 && -f "$scratch/controller.log" ]]; then
    cat "$scratch/controller.log" >&2
    cat "$scratch/worker.log" >&2
  fi
  if [[ "$created" == true ]]; then
    helm uninstall coding-validation --kube-context docker-desktop -n "$namespace" --ignore-not-found >/dev/null || status=1
    kubectl --context docker-desktop delete namespace "$namespace" --wait=false >/dev/null || status=1
    kubectl --context docker-desktop wait --for=delete namespace/"$namespace" --timeout=50s >/dev/null || status=1
  fi
  rm -rf "$scratch"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
source "$root/dev/lib/common.sh"
dev_build_chart_dependencies
if kubectl --context docker-desktop get namespace "$namespace" >/dev/null 2>&1; then echo 'fixture namespace already exists' >&2; exit 1; fi
AYNI_CODING_FIXTURE_DIRECTORY="$scratch" cargo test --locked -p ayni-orchestrator-control-plane export_coding_cutover_fixture -- --ignored
name=$(cat "$scratch/execution-name")
kubectl --context docker-desktop create namespace "$namespace"
created=true
kubectl --context docker-desktop -n "$namespace" create secret generic openai-api --from-literal=api-key=validation-placeholder
helm upgrade --install coding-validation "$root/dev/charts/ayni-orchestrator-local" \
  --kube-context docker-desktop -n "$namespace" --set observability.enabled=false \
  --set ayni-orchestrator.runtime.enabled=true --set ayni-orchestrator.runtime.verificationMode=events \
  --set-string "ayni-orchestrator.images.controlPlane=$1" --set-string "ayni-orchestrator.images.web=$3" \
  --set ayni-orchestrator.images.pullPolicy=Never --set-string "ayni-orchestrator.runtime.workerImage=$2" \
  --set-string 'ayni-orchestrator.runtime.reporting.endpoint=http://control-plane:8080/events/verification' \
  --set-string 'ayni-orchestrator.runtime.reporting.source=ayni://orchestrator/supervisors/validation'
kubectl --context docker-desktop -n "$namespace" rollout status deployment/control-plane --timeout=180s
kubectl --context docker-desktop -n "$namespace" scale deployment/control-plane --replicas=0
kubectl --context docker-desktop -n "$namespace" wait --for=delete pod -l app.kubernetes.io/name=ayni-orchestrator-control-plane --timeout=60s
cat > "$scratch/state.yaml" <<YAML
apiVersion: v1
kind: Pod
metadata: {name: coding-state}
spec:
  restartPolicy: Never
  automountServiceAccountToken: false
  securityContext: {runAsUser: 10001, runAsGroup: 10001, fsGroup: 10001}
  containers:
    - name: state
      image: $2
      imagePullPolicy: Never
      command: [/bin/sh, -c, sleep 600]
      securityContext: {allowPrivilegeEscalation: false, capabilities: {drop: [ALL]}}
      volumeMounts: [{name: state, mountPath: /state}]
  volumes: [{name: state, persistentVolumeClaim: {claimName: control-plane-state}}]
YAML
kubectl --context docker-desktop -n "$namespace" create -f "$scratch/state.yaml"
kubectl --context docker-desktop -n "$namespace" wait --for=condition=Ready pod/coding-state --timeout=60s
kubectl --context docker-desktop -n "$namespace" cp "$scratch/orchestrator.sqlite" coding-state:/state/orchestrator.sqlite
# Hold deletion only in this disposable namespace so early acceptance is observable.
for role in connection-controller runtime-controller; do
  kubectl --context docker-desktop -n "$namespace" get role "$role" -o json | python3 -c '
import json,sys
role=json.load(sys.stdin)
role["metadata"].pop("managedFields",None)
for rule in role["rules"]:
    if "jobs" in rule["resources"]: rule["verbs"]=[v for v in rule["verbs"] if v!="delete"]
print(json.dumps(role))' | kubectl --context docker-desktop -n "$namespace" apply -f -
done
kubectl --context docker-desktop -n "$namespace" create configmap "$name" \
  --from-file=worker.json="$scratch/worker.json" --from-file=coding-reporting.json="$scratch/coding-reporting.json" \
  --dry-run=client -o json | python3 -c 'import json,sys; o=json.load(sys.stdin); o["immutable"]=True; print(json.dumps(o))' | kubectl --context docker-desktop -n "$namespace" create -f -
cat > "$scratch/job.yaml" <<YAML
apiVersion: batch/v1
kind: Job
metadata: {name: ${name}}
spec:
  backoffLimit: 0
  activeDeadlineSeconds: 300
  template:
    spec:
      serviceAccountName: runtime-coding
      automountServiceAccountToken: false
      restartPolicy: Never
      securityContext: {runAsUser: 10001, runAsGroup: 10001, fsGroup: 10001}
      initContainers:
        - name: output
          image: $2
          imagePullPolicy: Never
          args: [prepare-output, ${name}]
          volumeMounts: [{name: output, mountPath: /outputs}]
      containers:
        - name: supervisor
          image: $2
          imagePullPolicy: Never
          command: [/bin/sh, -c]
          args: ['mkdir -p /workspace; /usr/local/bin/ayni-orchestrator-worker code; sleep $supervisor_delay']
          env: [{name: POD_UID, valueFrom: {fieldRef: {fieldPath: metadata.uid}}}]
          securityContext: {allowPrivilegeEscalation: false, readOnlyRootFilesystem: true, capabilities: {drop: [ALL]}}
          volumeMounts:
            - {name: workspace, mountPath: /workspace}
            - {name: request, mountPath: /request/worker.json, subPath: worker.json, readOnly: true}
            - {name: request, mountPath: /request/coding-reporting.json, subPath: coding-reporting.json, readOnly: true}
            - {name: identity, mountPath: /reporting, readOnly: true}
            - {name: output, mountPath: /output, subPath: ${name}}
      volumes:
        - {name: workspace, emptyDir: {}}
        - {name: request, configMap: {name: ${name}}}
        - {name: output, persistentVolumeClaim: {claimName: runtime-outputs}}
        - name: identity
          projected:
            defaultMode: 288
            sources: [{serviceAccountToken: {audience: ayni-orchestrator-verification, expirationSeconds: 600, path: token}}]
YAML
kubectl --context docker-desktop -n "$namespace" create -f "$scratch/job.yaml"
# The result is retained while the reporting service is deliberately stopped.
kubectl --context docker-desktop -n "$namespace" wait --for=condition=Ready pod -l "job-name=$name" --timeout=60s
pod=$(kubectl --context docker-desktop -n "$namespace" get pod -l "job-name=$name" -o jsonpath='{.items[0].metadata.name}')
for attempt in $(seq 1 20); do
  if kubectl --context docker-desktop -n "$namespace" exec "$pod" -- test -f /output/result.json; then break; fi
  sleep 1
done
if [[ "$mode" == retained ]]; then
  kubectl --context docker-desktop -n "$namespace" wait --for=condition=Complete "job/$name" --timeout=90s
fi
kubectl --context docker-desktop -n "$namespace" scale deployment/control-plane --replicas=1
kubectl --context docker-desktop -n "$namespace" rollout status deployment/control-plane --timeout=180s
# Allow reporting backoff and two controller passes before checkpointing.
sleep 20
kubectl --context docker-desktop -n "$namespace" logs deployment/control-plane --tail=50 > "$scratch/controller.log"
kubectl --context docker-desktop -n "$namespace" logs "$pod" -c supervisor --tail=50 > "$scratch/worker.log"
kubectl --context docker-desktop -n "$namespace" scale deployment/control-plane --replicas=0
kubectl --context docker-desktop -n "$namespace" wait --for=delete pod -l app.kubernetes.io/name=ayni-orchestrator-control-plane --timeout=60s
kubectl --context docker-desktop -n "$namespace" cp coding-state:/state/orchestrator.sqlite "$scratch/accepted.sqlite"
kubectl --context docker-desktop -n "$namespace" get job "$name" -o json > "$scratch/job-state.json"
python3 - "$scratch" "$mode" <<'PY'
import json,sqlite3,sys
from pathlib import Path
root=Path(sys.argv[1]);db=sqlite3.connect(root/'accepted.sqlite')
assert db.execute('SELECT version FROM clean_schema').fetchone()==(17,)
assert db.execute('SELECT terminal FROM coding_executions').fetchone()==(1,), (db.execute('SELECT terminal,last_sequence,stopped FROM coding_executions').fetchall(),db.execute('SELECT kind,decision FROM coding_inbox').fetchall())
assert db.execute("SELECT COUNT(*) FROM coding_inbox WHERE kind=2 AND decision='accepted'").fetchone()==(1,)
raw=db.execute("SELECT accepted_payload FROM coding_inbox WHERE kind=2").fetchone()[0]
fact=json.loads(raw)['data'];assert fact['applied'] is True
job=json.loads((root/'job-state.json').read_text())
assert job.get('status',{}).get('active' if sys.argv[2]=='live' else 'succeeded')==1
assert db.execute("SELECT COUNT(*) FROM coding_inbox WHERE kind=2 AND stored_at IS NOT NULL AND broker_sequence>0").fetchone()==(1,)
print('Final result accepted and published through real JetStream; delivery mode:',sys.argv[2])
PY
kubectl --context docker-desktop -n "$namespace" scale deployment/control-plane --replicas=1
kubectl --context docker-desktop -n "$namespace" rollout status deployment/control-plane --timeout=180s
sleep 5
kubectl --context docker-desktop -n "$namespace" scale deployment/control-plane --replicas=0
kubectl --context docker-desktop -n "$namespace" wait --for=delete pod -l app.kubernetes.io/name=ayni-orchestrator-control-plane --timeout=60s
kubectl --context docker-desktop -n "$namespace" cp coding-state:/state/orchestrator.sqlite "$scratch/restarted.sqlite"
python3 - "$scratch/restarted.sqlite" <<'PY'
import sqlite3,sys
c=sqlite3.connect(sys.argv[1]);assert c.execute("SELECT COUNT(*) FROM coding_inbox WHERE kind=2 AND decision='accepted'").fetchone()==(1,)
assert c.execute('SELECT terminal FROM coding_executions').fetchone()==(1,)
print('Restart preserves one terminal receipt, outcome and pending cleanup.')
PY
