#!/usr/bin/env bash
# Exercise genuine Pod-bound TokenReview and supervisor receipt in an isolated
# local-chart namespace. Synthetic retained evidence never activates a runtime.
set -euo pipefail
export HELM_NO_PLUGINS=1
root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
if [[ $# != 3 ]]; then
  echo 'usage: dev/tests/verification-reporting.sh CONTROL_PLANE_IMAGE PINNED_WORKER_IMAGE WEB_IMAGE' >&2
  exit 2
fi
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
  --set ayni-orchestrator.development.hotReload.enabled=true \
  --set ayni-orchestrator.runtime.enabled=true \
  --set-string "ayni-orchestrator.images.controlPlane=$1" \
  --set-string "ayni-orchestrator.images.web=$3" \
  --set ayni-orchestrator.images.pullPolicy=Never \
  --set-string "ayni-orchestrator.runtime.workerImage=$2" \
  --set-string 'ayni-orchestrator.runtime.reporting.endpoint=http://control-plane:8080/events/verification' \
  --set-string 'ayni-orchestrator.runtime.reporting.source=ayni://orchestrator/supervisors/validation'
kubectl --context docker-desktop -n "$namespace" rollout status deployment/control-plane --timeout=180s
python3 - "$root" "$temporary" "$job" "$2" <<'PY'
import hashlib,json,sys
from pathlib import Path
root,out,job,image=sys.argv[1:]
binding=json.loads((Path(root)/'proto/testdata/verification/report.json').read_text())['data']['binding']
binding.update(executionId=job,podUid='',requestDigest='sha256:'+hashlib.sha256(b'{}').hexdigest(),resolvedImageDigest=image.rsplit('@',1)[1])
admission={'configuration':{'source':'ayni://orchestrator/supervisors/validation','endpoint':'http://control-plane:8080/events/verification'},'binding':binding}
(Path(out)/'reporting.json').write_text(json.dumps(admission))
(Path(out)/'execution.json').write_text('{}')
PY
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
  activeDeadlineSeconds: 180
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
          image: $2
          imagePullPolicy: Never
          args: [prepare-output, $job]
          volumeMounts:
            - {name: output, mountPath: /outputs}
      containers:
        - name: verify
          image: $2
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
              echo verification-receipt-and-supervisor-replay-passed
          env:
            - name: JOB_NAME
              value: $job
            - name: POD_UID
              valueFrom: {fieldRef: {fieldPath: metadata.uid}}
            - name: REQUEST_DIGEST
              value: sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a
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
kubectl --context docker-desktop -n "$namespace" create -f "$temporary/job.yaml"
kubectl --context docker-desktop -n "$namespace" wait --for=condition=complete "job/$job" --timeout=180s
kubectl --context docker-desktop -n "$namespace" logs "job/$job" -c verify | tail -1
# Exercise the retained SQLite store on the same chart after a process/Pod restart.
kubectl --context docker-desktop -n "$namespace" rollout restart deployment/control-plane
kubectl --context docker-desktop -n "$namespace" rollout status deployment/control-plane --timeout=180s
# Keep the synthetic database and evidence for inspection; stop the idle deployment.
kubectl --context docker-desktop -n "$namespace" scale deployment/control-plane --replicas=0
printf 'reporting validation passed; retained synthetic state namespace=%s\n' "$namespace"
