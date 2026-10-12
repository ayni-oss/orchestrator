#!/usr/bin/env bash
# Exercise genuine Pod-bound TokenReview and supervisor receipt in an isolated
# local-chart namespace. Synthetic retained evidence never activates a runtime.
set -euo pipefail
export HELM_NO_PLUGINS=1
root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
if [[ $# != 3 && $# != 4 ]]; then
  echo 'usage: dev/tests/verification-reporting.sh CONTROL_PLANE_IMAGE PINNED_WORKER_IMAGE WEB_IMAGE [events]' >&2
  exit 2
fi
worker_image=$2
mode=${4:-shadow}
if [[ "$mode" != shadow && "$mode" != events ]]; then echo "unsupported validation mode" >&2; exit 2; fi
namespace="ayni-reporting-validation-$(date +%s)"
job="verification-receipt-smoke"
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT
source "$root/dev/lib/common.sh"
dev_build_chart_dependencies
kubectl --context docker-desktop create namespace "$namespace"
# Fixture credential only, never a copied operator/provider credential.
kubectl --context docker-desktop -n "$namespace" create secret generic openai-api --from-literal=api-key=validation-placeholder
helm upgrade --install reporting-validation "$root/dev/charts/ayni-orchestrator-local" \
  --kube-context docker-desktop --namespace "$namespace" \
  --set observability.enabled=false \
  --set ayni-orchestrator.runtime.enabled=true \
  --set-string "ayni-orchestrator.runtime.verificationMode=$mode" \
  --set-string "ayni-orchestrator.images.controlPlane=$1" \
  --set-string "ayni-orchestrator.images.web=$3" \
  --set ayni-orchestrator.images.pullPolicy=Never \
  --set-string "ayni-orchestrator.runtime.workerImage=$2" \
  --set-string 'ayni-orchestrator.runtime.reporting.endpoint=http://control-plane:8080/events/verification' \
  --set-string 'ayni-orchestrator.runtime.reporting.source=ayni://orchestrator/supervisors/validation'
kubectl --context docker-desktop -n "$namespace" rollout status deployment/control-plane --timeout=180s
if [[ "$mode" == events ]]; then
  AYNI_VERIFICATION_FIXTURE_DIRECTORY="$temporary" AYNI_VERIFICATION_FIXTURE_WORKER="$worker_image"     cargo test --locked -p ayni-orchestrator-control-plane export_verification_cutover_fixture -- --ignored
  kubectl --context docker-desktop -n "$namespace" scale deployment/control-plane --replicas=0
  kubectl --context docker-desktop -n "$namespace" wait --for=delete pod -l app.kubernetes.io/name=ayni-orchestrator-control-plane --timeout=60s
  cat > "$temporary/seed.yaml" <<YAML
apiVersion: v1
kind: Pod
metadata: {name: reporting-state-seed}
spec:
  restartPolicy: Never
  automountServiceAccountToken: false
  securityContext: {runAsUser: 10001, runAsGroup: 10001, fsGroup: 10001}
  containers:
    - name: seed
      image: $worker_image
      imagePullPolicy: Never
      command: [/bin/sh, -c, sleep 120]
      securityContext: {allowPrivilegeEscalation: false, capabilities: {drop: [ALL]}}
      volumeMounts: [{name: state, mountPath: /state}]
  volumes: [{name: state, persistentVolumeClaim: {claimName: control-plane-state}}]
YAML
  kubectl --context docker-desktop -n "$namespace" create -f "$temporary/seed.yaml"
  kubectl --context docker-desktop -n "$namespace" wait --for=condition=Ready pod/reporting-state-seed --timeout=60s
  # Only this new disposable namespace's empty state is replaced by admitted fixtures.
  kubectl --context docker-desktop -n "$namespace" cp "$temporary/orchestrator.sqlite" reporting-state-seed:/state/orchestrator.sqlite
  kubectl --context docker-desktop -n "$namespace" delete pod reporting-state-seed
  kubectl --context docker-desktop -n "$namespace" scale deployment/control-plane --replicas=1
  kubectl --context docker-desktop -n "$namespace" rollout status deployment/control-plane --timeout=180s
else
python3 - "$root" "$temporary" "$job" "$worker_image" <<'PY'
import hashlib,json,sys
from pathlib import Path
root,out,job,image=sys.argv[1:]
binding=json.loads((Path(root)/'proto/testdata/verification/report.json').read_text())['data']['binding']
binding.update(executionId=job,podUid='',requestDigest='sha256:'+hashlib.sha256(b'{}').hexdigest(),resolvedImageDigest=image.rsplit('@',1)[1])
admission={'configuration':{'source':'ayni://orchestrator/supervisors/validation','endpoint':'http://control-plane:8080/events/verification'},'binding':binding}
(Path(out)/'reporting.json').write_text(json.dumps(admission))
(Path(out)/'execution.json').write_text('{}')
PY
fi
request_digest=$(python3 - "$temporary/reporting.json" <<'PYDIGEST'
import json,sys
print(json.load(open(sys.argv[1]))['binding']['requestDigest'])
PYDIGEST
)
kubectl --context docker-desktop -n "$namespace" create configmap "$job" --from-file="reporting.json=$temporary/reporting.json" --from-file="execution.json=$temporary/execution.json" --dry-run=client -o json \
  | python3 -c 'import json,sys; o=json.load(sys.stdin); o["immutable"]=True; print(json.dumps(o))' \
  | kubectl --context docker-desktop -n "$namespace" create -f -
cat > "$temporary/job.yaml" <<YAML
apiVersion: batch/v1
kind: Job
metadata:
  name: $job
spec:
  backoffLimit: 0
  activeDeadlineSeconds: 300
  template:
    spec:
      restartPolicy: Never
      serviceAccountName: runtime-coding
      automountServiceAccountToken: false
      securityContext:
        runAsUser: 10001
        runAsGroup: 10001
        fsGroup: 10001
      initContainers:
        - name: output
          image: $worker_image
          imagePullPolicy: Never
          args: [prepare-output, $job]
          volumeMounts:
            - {name: output, mountPath: /outputs}
      containers:
        - name: verify
          image: $worker_image
          imagePullPolicy: Never
          command: [/bin/sh, -c]
          args:
            - |
              /usr/local/bin/ayni-orchestrator-worker verify
              result=\$?
              test "\$result" = 1 || exit 1
              /usr/local/bin/ayni-orchestrator-worker report-verification || exit 1
              test -s /output/verification-event.json || exit 1
              test -s /output/verification-receipt.json || exit 1
              echo verification-receipt-stored
              while test ! -f /output/replay-after-restart; do sleep 1; done
              mv /output/verification-receipt.json /output/verification-receipt-before-restart.json
              /usr/local/bin/ayni-orchestrator-worker report-verification || exit 1
              cmp /output/verification-receipt-before-restart.json /output/verification-receipt.json || exit 1
              echo verification-receipt-and-supervisor-replay-passed
          env:
            - name: JOB_NAME
              value: $job
            - name: POD_UID
              valueFrom: {fieldRef: {fieldPath: metadata.uid}}
            - name: REQUEST_DIGEST
              value: $request_digest
          volumeMounts:
            - {name: request, mountPath: /request/execution.json, subPath: execution.json, readOnly: true}
            - {name: request, mountPath: /request/reporting.json, subPath: reporting.json, readOnly: true}
            - {name: identity, mountPath: /reporting, readOnly: true}
            - {name: output, mountPath: /output, subPath: $job}
      volumes:
        - name: request
          configMap: {name: $job}
        - name: output
          persistentVolumeClaim: {claimName: runtime-outputs}
        - name: identity
          projected:
            defaultMode: 288
            sources:
              - serviceAccountToken: {audience: ayni-orchestrator-verification, expirationSeconds: 600, path: token}
YAML
inspect_state() {
cat > "$temporary/inspect.yaml" <<YAML
apiVersion: v1
kind: Pod
metadata:
  name: reporting-inbox-inspector
spec:
  restartPolicy: Never
  automountServiceAccountToken: false
  securityContext: {runAsUser: 10001, runAsGroup: 10001, fsGroup: 10001}
  containers:
    - name: inspector
      image: $worker_image
      imagePullPolicy: Never
      command: [/bin/sh, -c, sleep 120]
      securityContext:
        allowPrivilegeEscalation: false
        capabilities: {drop: [ALL]}
        readOnlyRootFilesystem: true
      volumeMounts:
        - {name: state, mountPath: /state, readOnly: true}
  volumes:
    - name: state
      persistentVolumeClaim: {claimName: control-plane-state, readOnly: true}
YAML
kubectl --context docker-desktop -n "$namespace" create -f "$temporary/inspect.yaml"
kubectl --context docker-desktop -n "$namespace" wait --for=condition=Ready pod/reporting-inbox-inspector --timeout=60s
kubectl --context docker-desktop -n "$namespace" cp reporting-inbox-inspector:/state/orchestrator.sqlite "$temporary/synthetic.sqlite"
kubectl --context docker-desktop -n "$namespace" delete pod/reporting-inbox-inspector
python3 - "$temporary/synthetic.sqlite" "$mode" <<'PYSQL'
import json,sqlite3,sys
connection=sqlite3.connect('file:'+sys.argv[1]+'?mode=ro',uri=True)
assert connection.execute("SELECT version FROM clean_schema").fetchone()==(13,)
if sys.argv[2]=='events':
    assert connection.execute("SELECT execution_id,decision FROM verification_inbox").fetchall()==[('verification-receipt-smoke','accepted')]
    cursor=json.loads(connection.execute("SELECT value FROM reconciliation_cursors WHERE name LIKE 'managed:%'").fetchone()[0])
    assert cursor['failures']==1 and cursor['pending']['failed'] is True
else:
    assert connection.execute("SELECT execution_id,decision,reason FROM verification_inbox").fetchall()==[('verification-receipt-smoke','rejected','verification_authority_stale')]
assert connection.execute("SELECT COUNT(*) FROM verification_journal").fetchone()==(0,)
assert connection.execute("SELECT COUNT(*) FROM verification_outbox").fetchone()==(0,)
print('durable verification decision and failure accounting verified')
PYSQL
}
kubectl --context docker-desktop -n "$namespace" create -f "$temporary/job.yaml"
# Keep the original physical Pod alive across the control-plane restart. The
# fixture preserves its first receipt, then forces a real duplicate HTTP replay.
pod=""
for attempt in $(seq 1 60); do
  pod=$(kubectl --context docker-desktop -n "$namespace" get pods -l "job-name=$job" -o jsonpath='{.items[*].metadata.name}')
  if [[ -n "$pod" ]]; then break; fi
  sleep 1
done
if [[ -z "$pod" || "$pod" == *" "* ]]; then
  echo "expected exactly one supervisor Pod" >&2
  exit 1
fi
received=false
for attempt in $(seq 1 120); do
  if kubectl --context docker-desktop -n "$namespace" exec "$pod" -c verify -- test -s /output/verification-receipt.json 2>/dev/null; then
    received=true
    break
  fi
  sleep 1
done
if [[ "$received" != true ]]; then
  echo "supervisor did not retain a receipt in time" >&2
  exit 1
fi
if [[ "$mode" == events ]]; then
  sleep 6
  kubectl --context docker-desktop -n "$namespace" scale deployment/control-plane --replicas=0
  kubectl --context docker-desktop -n "$namespace" wait --for=delete pod -l app.kubernetes.io/name=ayni-orchestrator-control-plane --timeout=60s
  test "$(kubectl --context docker-desktop -n "$namespace" get pod "$pod" -o jsonpath='{.status.phase}')" = Running
  inspect_state
  echo authoritative-failure-accepted-before-supervisor-exit
  kubectl --context docker-desktop -n "$namespace" scale deployment/control-plane --replicas=1
fi
kubectl --context docker-desktop -n "$namespace" rollout restart deployment/control-plane
kubectl --context docker-desktop -n "$namespace" rollout status deployment/control-plane --timeout=180s
kubectl --context docker-desktop -n "$namespace" exec "$pod" -c verify -- touch /output/replay-after-restart
kubectl --context docker-desktop -n "$namespace" wait --for=condition=complete "job/$job" --timeout=120s
kubectl --context docker-desktop -n "$namespace" logs "job/$job" -c verify | tail -1
# Allow the bounded periodic consumer to decide the synthetic unselected receipt.
sleep 6
# Keep the synthetic database and evidence for inspection; stop the idle deployment.
kubectl --context docker-desktop -n "$namespace" scale deployment/control-plane --replicas=0
kubectl --context docker-desktop -n "$namespace" wait --for=delete pod -l app.kubernetes.io/name=ayni-orchestrator-control-plane --timeout=60s
inspect_state
printf 'reporting validation passed; retained synthetic state namespace=%s\n' "$namespace"
