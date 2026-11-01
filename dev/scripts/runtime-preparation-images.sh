#!/usr/bin/env bash
# Local Desktop packaging. Never installs a release or marks a profile qualified.
set -euo pipefail
if [[ $# -lt 2 || $# -gt 3 ]]; then
  echo "usage: $0 <patched-ayni-linux-arm64> <expected-cli-sha256> [static-binaries-directory]" >&2
  exit 2
fi
cli_path="$1"
cli_digest="${2#sha256:}"
[[ "$cli_digest" =~ ^[a-f0-9]{64}$ ]] || { echo "invalid CLI checksum" >&2; exit 2; }
[[ -f "$cli_path" ]] || { echo "CLI artifact unavailable" >&2; exit 2; }
DEV_REPOSITORY_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
export DEV_REPOSITORY_ROOT
context="$(mktemp -d "${TMPDIR:-/tmp}/ayni-runtime-images.XXXXXX")"
build_pid=""
cleanup() {
  [[ -z "$build_pid" ]] || kill -TERM "$build_pid" 2>/dev/null || true
  rm -rf "$context"
}
trap cleanup EXIT
trap 'exit 130' INT TERM
mkdir "$context/docker-config"
printf '%s\n' '{"auths":{},"cliPluginsExtraDirs":["/Applications/Docker.app/Contents/Resources/cli-plugins"]}' > "$context/docker-config/config.json"
unset DOCKER_CONTEXT
export DOCKER_CONFIG="$context/docker-config" DOCKER_HOST="unix://${HOME}/.docker/run/docker.sock"
case "${AYNI_RUNTIME_KUBE_CONTEXT:-docker-desktop}" in
  docker-desktop) node_name=desktop-control-plane; cluster_name=desktop ;;
  kind-ayni-runtime-qualification) node_name=ayni-runtime-qualification-control-plane; cluster_name=ayni-runtime-qualification ;;
  *) echo "unsupported explicit runtime cluster context" >&2; exit 2 ;;
esac
load_runtime_image() {
  local reference="$1" labels
  labels="$(docker inspect "$node_name" --format '{{index .Config.Labels "io.x-k8s.kind.cluster"}} {{index .Config.Labels "io.x-k8s.kind.role"}}')"
  [[ "$labels" == "$cluster_name control-plane" ]] || { echo "runtime node identity mismatch" >&2; return 1; }
  docker save --platform linux/arm64 "$reference" \
    | docker exec -i "$node_name" ctr --namespace k8s.io images import --platform linux/arm64 - >&2
}
# Ten GiB is a conservative cancellation threshold, preserving eight GiB while
# BuildKit responds to cancellation. This is a host guard, not a workload quota.
headroom() {
  local free_kib
  free_kib="$(df -k "$DEV_REPOSITORY_ROOT" | awk 'NR==2{print $4}')"
  (( free_kib >= 10485760 ))
}
guarded() {
  headroom || { echo "at least 10 GiB host headroom is required" >&2; return 1; }
  "$@" & build_pid=$!
  while kill -0 "$build_pid" 2>/dev/null; do
    if ! headroom; then
      echo "host disk guard cancelled this build" >&2
      kill -TERM "$build_pid" 2>/dev/null || true
      wait "$build_pid" || true
      build_pid=""
      return 1
    fi
    sleep 2
  done
  local status=0
  wait "$build_pid" || status=$?
  build_pid=""
  return "$status"
}
if [[ $# == 3 ]]; then
  for binary in runtime-worker runtime-supervisor runtime-verifier; do cp "$3/$binary" "$context/$binary"; done
else
  guarded docker buildx build --platform linux/arm64 \
    --file "$DEV_REPOSITORY_ROOT/deploy/images/runtime-preparation/Dockerfile.binaries" \
    --target binaries --output "type=local,dest=$context" "$DEV_REPOSITORY_ROOT"
fi
for binary in runtime-worker runtime-supervisor runtime-verifier; do
  file "$context/$binary" | grep -Eq 'ELF 64-bit.*(ARM aarch64|aarch64).*static' || { echo "$binary must be a static Linux arm64 executable" >&2; exit 2; }
done
cp "$cli_path" "$context/ayni"
printf '%s\n' "$cli_digest  $context/ayni" | shasum -a 256 -c -
curl --fail --location --proto '=https' --tlsv1.2 https://github.com/opencontainers/umoci/releases/download/v0.6.0/umoci.linux.arm64 --output "$context/umoci.linux.arm64"
printf '%s\n' "5cfd17f2e7a4bcf9ed67ea1b955ca893d200349b9ce6a3d3707dba415f458a1f  $context/umoci.linux.arm64" | shasum -a 256 -c -
for kind in worker probes; do
  dockerfile="$DEV_REPOSITORY_ROOT/deploy/images/runtime-preparation/Dockerfile"
  [[ "$kind" != probes ]] || dockerfile="$dockerfile.probes"
  image="local/ayni-runtime-$kind:qualification-$(date +%s)-$$"
  guarded docker build --platform linux/arm64 --file "$dockerfile" --build-arg "AYNI_CLI_SHA256=$cli_digest" --tag "$image" "$context"
  load_runtime_image "$image"
  node_manifest="$(docker exec "$node_name" ctr --namespace k8s.io images list | awk -v reference="docker.io/$image" '$1 == reference { print $3 }')"
  [[ "$node_manifest" =~ ^sha256:[a-f0-9]{64}$ ]] || { echo "imported node manifest unavailable" >&2; exit 2; }
  immutable="docker.io/${image%:*}@$node_manifest"
  existing_manifest="$(docker exec "$node_name" ctr --namespace k8s.io images list | awk -v reference="$immutable" '$1 == reference { print $3 }')"
  if [[ -n "$existing_manifest" && "$existing_manifest" != "$node_manifest" ]]; then
    echo "immutable node alias conflicts with imported manifest" >&2
    exit 2
  fi
  if [[ -z "$existing_manifest" ]]; then
    docker exec "$node_name" ctr --namespace k8s.io images tag "docker.io/$image" "$immutable" >/dev/null
  fi
  echo "$kind=$image"
  echo "${kind}_immutable=$immutable"
done
