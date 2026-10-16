#!/usr/bin/env bash
# Real pinned Ayni lock regeneration inside the worker's sandbox on Docker Desktop.
# Exercises CLI/proposal boundaries; does not claim model/provider task execution
# or certification of a new runtime image.
set -euo pipefail
[[ $# == 2 ]] || { echo 'usage: dev/tests/environment-changes.sh WORKER_IMAGE RUNTIME_IMAGE' >&2; exit 2; }
namespace="ayni-environment-change-$$"
scratch=$(mktemp -d)
created=false
k() { kubectl --context docker-desktop -n "$namespace" "$@"; }
cleanup() {
  status=$?
  if [[ "$created" == true ]]; then
    if [[ "$status" != 0 ]]; then k describe pod/environment-change >&2 || true; k logs environment-change >&2 || true; fi
    kubectl --context docker-desktop delete namespace "$namespace" --wait=false >/dev/null || status=1
    kubectl --context docker-desktop wait --for=delete "namespace/$namespace" --timeout=60s >/dev/null || status=1
  fi
  rm -rf "$scratch"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
kubectl --context docker-desktop get nodes >/dev/null
kubectl --context docker-desktop create namespace "$namespace"
created=true
cat > "$scratch/check.sh" <<'SCRIPT'
#!/bin/sh
set -eu
cd /workspace
export HOME=/scratch/home MISE_DATA_DIR=/scratch/mise-data MISE_CONFIG_DIR=/scratch/mise-config
mkdir -p "$HOME" src
cli() { /launcher/ayni-orchestrator-worker sandbox /usr/local/bin/ayni "$@"; }
[ "$(cli --version)" = 'ayni 0.15.0' ]
test -f /etc/ayni/runtime.json
runtime_before=$(sha256sum /etc/ayni/runtime.json)
cat > .ayni.toml <<'POLICY'
[checks]
test = false
coverage = false
size = true
complexity = false
deps = false
mutation = false
[languages]
enabled = ["rust"]
[rust]
roots = ["."]
[rust.size]
"*.rs" = { warn = 100, fail = 200 }
POLICY
cat > Cargo.toml <<'MANIFEST'
[package]
name = "reviewed-environment-fixture"
version = "0.1.0"
edition = "2024"
MANIFEST
cat > rust-toolchain.toml <<'TOOLCHAIN'
[toolchain]
channel = "1.94.0"
TOOLCHAIN
printf 'fn main() {}\n' > src/main.rs
cat > Cargo.lock <<'LOCK'
version = 4
[[package]]
name = "reviewed-environment-fixture"
version = "0.1.0"
LOCK
base='docker.io/library/debian:bookworm-slim@sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241'
policy_before=$(sha256sum .ayni.toml)
cli env lock --base "$base"
cp .ayni.lock baseline.lock
sed -i 's/0.1.0/0.2.0/g' Cargo.toml Cargo.lock
cli env lock --base "$base"
if cmp -s baseline.lock .ayni.lock; then echo 'dependency changes did not alter derived lock' >&2; exit 1; fi
cp .ayni.lock proposed.lock
cli env lock --base "$base"
cmp proposed.lock .ayni.lock
test "$policy_before" = "$(sha256sum .ayni.toml)"
test "$runtime_before" = "$(sha256sum /etc/ayni/runtime.json)"
printf 'baseline_lock_digest='; sha256sum baseline.lock
printf 'proposed_lock_digest='; sha256sum proposed.lock
printf 'baseline_runtime_metadata_digest='; sha256sum /etc/ayni/runtime.json
printf 'policy_digest='; sha256sum .ayni.toml
printf 'lock_regeneration=succeeded local_dependency_preparation=unavailable proposed_runtime_certification=not_attempted\n'
SCRIPT
k create configmap environment-change-script --from-file=check.sh="$scratch/check.sh"
cat > "$scratch/pod.yaml" <<YAML
apiVersion: v1
kind: Pod
metadata: {name: environment-change}
spec:
  restartPolicy: Never
  automountServiceAccountToken: false
  securityContext: {runAsNonRoot: true, runAsUser: 10001, runAsGroup: 10001, fsGroup: 10001, seccompProfile: {type: RuntimeDefault}}
  initContainers:
    - name: launcher
      image: $1
      imagePullPolicy: Never
      command: [/bin/sh, -c, 'cp /usr/local/bin/ayni-orchestrator-worker /launcher/ayni-orchestrator-worker']
      securityContext: {allowPrivilegeEscalation: false, readOnlyRootFilesystem: true, capabilities: {drop: [ALL]}}
      volumeMounts: [{name: launcher, mountPath: /launcher}]
  containers:
    - name: check
      image: $2
      imagePullPolicy: Never
      command: [/bin/sh, /fixture/check.sh]
      securityContext: {allowPrivilegeEscalation: false, readOnlyRootFilesystem: true, capabilities: {drop: [ALL]}}
      resources: {requests: {cpu: 100m, memory: 256Mi}, limits: {cpu: '2', memory: 2Gi}}
      volumeMounts: [{name: launcher, mountPath: /launcher, readOnly: true}, {name: fixture, mountPath: /fixture, readOnly: true}, {name: workspace, mountPath: /workspace}, {name: scratch, mountPath: /scratch}, {name: tmp, mountPath: /tmp}]
  volumes:
    - {name: launcher, emptyDir: {}}
    - {name: fixture, configMap: {name: environment-change-script}}
    - {name: workspace, emptyDir: {}}
    - {name: scratch, emptyDir: {}}
    - {name: tmp, emptyDir: {}}
YAML
k create -f "$scratch/pod.yaml"
for _ in {1..120}; do
  phase=$(k get pod environment-change -o jsonpath='{.status.phase}')
  case "$phase" in
    Succeeded) k logs environment-change; exit 0 ;;
    Failed) k logs environment-change >&2; exit 1 ;;
  esac
  sleep 1
done
echo 'environment-change fixture timed out' >&2
exit 1
