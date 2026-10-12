#!/usr/bin/env bash
# Disposable local transport proof; cleans only its dedicated test namespace.
set -euo pipefail
root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
source "$root/dev/lib/common.sh"
context=$(dev_context)
namespace=ayni-orchestrator-jetstream-test
created_namespace=false
scratch=$(mktemp -d)
mkdir -p "$scratch/plugins"
export HELM_PLUGINS="$scratch/plugins"
prometheus_pid='' grafana_pid=''
cleanup() {
  for pid in "$prometheus_pid" "$grafana_pid"; do
    if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
  done
  rm -rf "$scratch"
  if [[ "$created_namespace" == true ]]; then
    # Helm removes cluster-scoped monitoring RBAC; namespace deletion removes
    # the intentionally retained test PVC. Both are confined to this fixture.
    helm uninstall "$DEV_RELEASE_NAME" --kube-context "$context" --namespace "$namespace" --ignore-not-found
    kubectl --context "$context" delete namespace "$namespace" --wait=false
    kubectl --context "$context" wait --for=delete "namespace/$namespace" --timeout=50s
    echo "Removed disposable test namespace $namespace and its test PVC."
  fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
dev_build_chart_dependencies
# Refuse to overwrite an existing environment, including an interrupted fixture.
if kubectl --context "$context" get namespace "$namespace" >/dev/null 2>&1; then
  echo "Test namespace already exists: $namespace; inspect and clean it before rerunning" >&2
  exit 1
fi
kubectl --context "$context" create namespace "$namespace"
created_namespace=true
kubectl --context "$context" label namespace "$namespace" orchestrator.ayni.dev/environment=local
install() {
  helm upgrade --install "$DEV_RELEASE_NAME" "$DEV_CHART" \
    --kube-context "$context" --namespace "$namespace" --wait --timeout 5m \
    --set ayni-orchestrator.enabled=false --set observability.enabled=true \
    --set jetstream.lifecycle.maxMessages="$1"
}
install 3
image=$(helm get values "$DEV_RELEASE_NAME" --all --output json --kube-context "$context" \
  --namespace "$namespace" | python3 -c 'import json,sys; print(json.load(sys.stdin)["jetstream"]["toolingImage"])')
kubectl --context "$context" --namespace "$namespace" run jetstream-test --image="$image" \
  --labels=orchestrator.ayni.dev/jetstream-client=true --restart=Never \
  --overrides='{"spec":{"automountServiceAccountToken":false,"securityContext":{"runAsNonRoot":true,"runAsUser":10001,"runAsGroup":10001,"seccompProfile":{"type":"RuntimeDefault"}},"containers":[{"name":"jetstream-test","image":"'"$image"'","command":["sleep","3600"],"workingDir":"/tmp","resources":{"requests":{"cpu":"10m","memory":"32Mi"},"limits":{"cpu":"100m","memory":"64Mi"}},"securityContext":{"allowPrivilegeEscalation":false,"readOnlyRootFilesystem":true,"capabilities":{"drop":["ALL"]}}}]}}'
kubectl --context "$context" --namespace "$namespace" wait --for=condition=Ready pod/jetstream-test --timeout=120s
nats() {
  kubectl --context "$context" --namespace "$namespace" exec jetstream-test -- \
    nats --server=nats://jetstream:4222 --timeout=5s "$@"
}
assert_first() {
  nats stream get "$stream" 1 --json >"$scratch/message.json"
  python3 - "$scratch/message.json" <<'PYMSG'
import base64, json, sys
x = json.load(open(sys.argv[1]))
message = x.get('message', x)
assert base64.b64decode(message['data']) == b'first', x
PYMSG
}
subject=orchestrator.runtime.verification.accepted.v1
stream=ORCHESTRATOR_LIFECYCLE
nats pub --jetstream "$subject" first --header=Nats-Msg-Id:fixture-first
nats pub --jetstream "$subject" first --header=Nats-Msg-Id:fixture-first
nats stream info "$stream" --json >"$scratch/info.json"
python3 - "$scratch/info.json" <<'PY'
import json, sys
x = json.load(open(sys.argv[1]))
assert x['state']['messages'] == 1, x
assert x['config']['storage'] == 'file' and x['config']['retention'] == 'limits'
assert x['config']['discard'] == 'new' and x['config']['max_msg_size'] == 16384
assert x['config']['max_age'] == 604800000000000
assert x['config']['duplicate_window'] == 120000000000
assert x['config']['max_consumers'] == 8
PY
# Repeated provisioning must edit in place, retaining the original sequence/data.
install 3
assert_first
kubectl --context "$context" --namespace "$namespace" rollout restart statefulset/jetstream
kubectl --context "$context" --namespace "$namespace" rollout status statefulset/jetstream --timeout=120s
assert_first
nats pub --jetstream "$subject" second --header=Nats-Msg-Id:fixture-second
nats pub --jetstream "$subject" third --header=Nats-Msg-Id:fixture-third
if nats pub --jetstream "$subject" fourth --header=Nats-Msg-Id:fixture-fourth >"$scratch/full.log" 2>&1; then
  echo 'Full lifecycle stream accepted a new message' >&2; exit 1
fi
grep -Ei 'maximum messages|limit|full' "$scratch/full.log"
assert_first
# Recover by expanding the explicit limit; retry the same rejected identity.
install 4
nats pub --jetstream "$subject" fourth --header=Nats-Msg-Id:fixture-fourth
nats stream info "$stream" --json >"$scratch/info.json"
python3 - "$scratch/info.json" <<'PY'
import json, sys
x = json.load(open(sys.argv[1]))
assert x['state']['messages'] == 4 and x['state']['first_seq'] == 1, x
PY
kubectl --context "$context" --namespace "$namespace" port-forward --address 127.0.0.1 \
  service/prometheus 19090:9090 >"$scratch/prometheus.log" 2>&1 &
prometheus_pid=$!
kubectl --context "$context" --namespace "$namespace" port-forward --address 127.0.0.1 \
  service/grafana 19000:3000 >"$scratch/grafana.log" 2>&1 &
grafana_pid=$!
for attempt in {1..30}; do
  if curl -fsS --max-time 5 --get --data-urlencode 'query=jetstream_stream_total_messages{stream_name="ORCHESTRATOR_LIFECYCLE"}' \
    http://127.0.0.1:19090/api/v1/query >"$scratch/metrics.json" && \
    python3 - "$scratch/metrics.json" <<'PY'
import json, sys
x = json.load(open(sys.argv[1]))
assert any(float(v['value'][1]) == 4 for v in x['data']['result']), x
PY
  then break; fi
  sleep 2
done
python3 - "$scratch/metrics.json" <<'PYVERIFY'
import json, sys
x = json.load(open(sys.argv[1]))
assert any(float(v['value'][1]) == 4 for v in x['data']['result']), x
PYVERIFY
curl -fsS --max-time 10 http://127.0.0.1:19000/api/dashboards/uid/ayni-orchestrator-jetstream >"$scratch/dashboard.json"
python3 - "$scratch/dashboard.json" <<'PY'
import json, sys
x = json.load(open(sys.argv[1]))
assert x['dashboard']['uid'] == 'ayni-orchestrator-jetstream'
assert len(x['dashboard']['panels']) == 13
with open(sys.argv[1] + '.queries', 'w') as f:
    for panel in x['dashboard']['panels'][:6]:
        f.write(panel['targets'][0]['expr'] + '\n')
PY
# Query through Grafana's provisioned datasource as well as direct Prometheus.
while IFS= read -r query; do
  curl -fsS --max-time 10 --get --data-urlencode "query=$query" \
    http://127.0.0.1:19000/api/datasources/proxy/uid/prometheus/api/v1/query >"$scratch/query.json"
  python3 - "$scratch/query.json" <<'PYQUERY'
import json, sys
x = json.load(open(sys.argv[1]))
assert x['status'] == 'success' and x['data']['result'], x
PYQUERY
done <"$scratch/dashboard.json.queries"
echo 'JetStream provisioning, restart persistence, deduplication, saturation recovery and dashboard passed.'
