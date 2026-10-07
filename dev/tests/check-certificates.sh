#!/usr/bin/env bash
# Synthetic-key smoke test in a new namespace, never an existing deployment.
set -euo pipefail
readonly image="${ORCHESTRATOR_CERTIFICATE_TEST_IMAGE:?set the newly built control-plane image}"
readonly port="${ORCHESTRATOR_CERTIFICATE_TEST_PORT:-15061}"
readonly seed="9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60"
readonly public="d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
readonly temp="$(mktemp -d /tmp/ayni-certificates-test.XXXXXX)"
readonly namespace="ayni-certificates-test-$(date +%s)-$(tr '[:upper:]' '[:lower:]' <<<"${temp##*.}")"
namespace_created=false
forward_pid=''
cleanup() {
  if [[ -n "$forward_pid" ]]; then kill "$forward_pid" 2>/dev/null || true; fi
  if [[ "$namespace_created" == true ]]; then
    kubectl --context docker-desktop delete namespace "$namespace" --wait=false >/dev/null || true
  fi
  rm -rf "$temp"
}
trap cleanup EXIT
kubectl --context docker-desktop create namespace "$namespace" >/dev/null
namespace_created=true
kubectl --context docker-desktop -n "$namespace" apply -f - <<YAML >/dev/null
apiVersion: v1
kind: ServiceAccount
metadata: {name: certificate-test}
---
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata: {name: certificate-test}
rules:
- apiGroups: [""]
  resources: [secrets]
  verbs: [get, create, delete]
- apiGroups: [batch]
  resources: [jobs]
  verbs: [list]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata: {name: certificate-test}
roleRef: {apiGroup: rbac.authorization.k8s.io, kind: Role, name: certificate-test}
subjects: [{kind: ServiceAccount, name: certificate-test}]
---
apiVersion: v1
kind: Pod
metadata: {name: control-plane}
spec:
  serviceAccountName: certificate-test
  containers:
  - name: control-plane
    image: $image
    imagePullPolicy: Never
    env:
    - {name: ORCHESTRATOR_BIND_ADDRESS, value: '0.0.0.0:8080'}
    - {name: ORCHESTRATOR_GRPC_BIND_ADDRESS, value: '127.0.0.1:50051'}
    - {name: ORCHESTRATOR_DATABASE_PATH, value: /data/test.sqlite}
    - {name: ORCHESTRATOR_CONNECTION_NAMESPACE, value: '$namespace'}
    - {name: ORCHESTRATOR_ADMIN_CERTIFICATES, value: '[{"id":"admin-test","displayName":"Administrator test","secretName":"synthetic-admin"}]'}
    volumeMounts: [{name: data, mountPath: /data}]
  volumes: [{name: data, emptyDir: {}}]
YAML
kubectl --context docker-desktop -n "$namespace" wait --for=condition=Ready pod/control-plane --timeout=180s >/dev/null
kubectl --context docker-desktop -n "$namespace" port-forward pod/control-plane "$port:50051" >"$temp/forward.log" 2>&1 &
forward_pid="$!"
api() {
  local method="$1" payload="$2" result
  result="$(grpcurl -plaintext -max-time 10 -d "$payload" "127.0.0.1:$port" "ayni.orchestrator.v1.CertificateService/$method")" || return $?
  if [[ "$result" == *"$seed"* || "$result" == *'signingSeed'* || "$result" == *'signing-key'* ]]; then
    echo 'private material reflected in API response' >&2; exit 1
  fi
  printf '%s' "$result"
}
forward_ready=false
for _ in {1..30}; do
  if grpcurl -plaintext -max-time 2 "127.0.0.1:$port" list >/dev/null 2>&1; then
    forward_ready=true
    break
  fi
  kill -0 "$forward_pid" 2>/dev/null || { echo 'certificate test port-forward failed' >&2; exit 1; }
  sleep 1
done
[[ "$forward_ready" == true ]] || { echo 'certificate test API did not become ready' >&2; exit 1; }
initial="$(api ListCertificates '{}')" || exit $?
[[ "$(jq '[.certificates[]? | select(.isDefault==true)] | length' <<<"$initial")" == 0 ]]
generated="$(api CreateCertificate '{"idempotencyKey":"smoke-generate-one","displayName":"Generated","keyId":"generated","generate":true}')"
id="$(jq -r '.certificate.id' <<<"$generated")"
[[ "$(jq -r '.certificate.isDefault' <<<"$generated")" == true ]]
retry="$(api CreateCertificate '{"idempotencyKey":"smoke-generate-one","displayName":"Generated","keyId":"generated","generate":true}')"
[[ "$(jq -r '.certificate.publicKey' <<<"$retry")" == "$(jq -r '.certificate.publicKey' <<<"$generated")" ]]
imported="$(api CreateCertificate "{\"idempotencyKey\":\"smoke-import-one\",\"displayName\":\"Imported\",\"keyId\":\"imported\",\"signingSeed\":\"$seed\"}")"
[[ "$(jq -r '.certificate.publicKey' <<<"$imported")" == "$public" ]]
other_id="$(jq -r '.certificate.id' <<<"$imported")"
api SetDefaultCertificate "{\"id\":\"$other_id\",\"idempotencyKey\":\"smoke-default-one\",\"expectedResourceVersion\":\"1\"}" >/dev/null
replaced="$(api ReplaceCertificate "{\"id\":\"$other_id\",\"idempotencyKey\":\"smoke-replace-one\",\"expectedResourceVersion\":\"2\",\"keyId\":\"rotated\",\"generate\":true}")"
[[ "$(jq -r '.certificate.publicKey' <<<"$replaced")" != "$public" ]]
[[ "$(jq -r '.certificate.retiredSecretRevisions | length' <<<"$replaced")" == 1 ]]
api DeleteCertificate "{\"id\":\"$id\",\"idempotencyKey\":\"smoke-delete-one\",\"expectedResourceVersion\":\"2\"}" >/dev/null
# Source Secret contains only deterministic synthetic test material.
kubectl --context docker-desktop -n "$namespace" create secret generic synthetic-admin --from-literal="signing-key=$seed" --from-literal=key-id=admin >/dev/null
for _ in {1..45}; do
  admin="$(api GetCertificate '{"id":"admin-test"}')"
  if [[ "$(jq -r '.certificate.status' <<<"$admin")" == ready ]]; then break; fi
  sleep 1
done
[[ "$(jq -r '.certificate.publicKey' <<<"$admin")" == "$public" ]]
[[ "$(jq -r '.certificate.secretName' <<<"$admin")" != synthetic-admin ]]
if api ReplaceCertificate "{\"id\":\"admin-test\",\"idempotencyKey\":\"smoke-admin-replace\",\"expectedResourceVersion\":\"$(jq -r '.certificate.resourceVersion' <<<"$admin")\",\"keyId\":\"new\",\"generate\":true}" 2>"$temp/rejected.log"; then
  echo 'administrator certificate accepted replacement' >&2; exit 1
fi
grep -q PermissionDenied "$temp/rejected.log"
printf '%s\n' 'certificate generation, import, rotation, retry, defaults, deletion, and administrator snapshot smoke checks passed'
