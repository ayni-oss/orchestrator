#!/usr/bin/env bash
# Create only the separate policy-enforcing qualification cluster. No resets.
set -euo pipefail
[[ $# == 1 ]] || { echo "usage: $0 <new-absolute-state-directory>" >&2; exit 2; }
state="$1"
[[ "$state" == /* && ! -e "$state" ]] || { echo "state directory must be new and absolute" >&2; exit 2; }
[[ "$(uname -sm)" == 'Darwin arm64' ]] || { echo "this local tool pin supports macOS arm64" >&2; exit 2; }
root="$(cd "$(dirname "$0")/../.." && pwd)"
mkdir -p "$state/docker-config" "$state/helm-plugins"
printf '%s\n' '{"auths":{},"cliPluginsExtraDirs":["/Applications/Docker.app/Contents/Resources/cli-plugins"]}' > "$state/docker-config/config.json"
printf '%s\n' '{"auths":{}}' > "$state/helm-registry.json"
unset DOCKER_CONTEXT
export DOCKER_CONFIG="$state/docker-config" DOCKER_HOST="unix://${HOME}/.docker/run/docker.sock"
export KIND_EXPERIMENTAL_PROVIDER=docker KIND_EXPERIMENTAL_DOCKER_NETWORK=ayni-runtime-qualification
export HELM_NO_PLUGINS=1 HELM_PLUGINS="$state/helm-plugins" HELM_REGISTRY_CONFIG="$state/helm-registry.json"
if docker inspect ayni-runtime-qualification-control-plane >/dev/null 2>&1 \
  || docker network inspect ayni-runtime-qualification >/dev/null 2>&1; then
  echo "refusing an existing qualification node or network; never resets" >&2; exit 2
fi
headroom() { (( $(df -k "$root" | awk 'NR==2{print $4}') >= 9437184 )); }
headroom || { echo "at least nine GiB host headroom is required" >&2; exit 2; }
curl --fail --location --proto '=https' --tlsv1.2 https://github.com/kubernetes-sigs/kind/releases/download/v0.33.0/kind-darwin-arm64 --output "$state/kind"
printf '%s\n' "0c8c7dbe5e23594a198b786c4bc13dacc101fa6196b0cb0b23a1ca44e61f4b4f  $state/kind" | shasum -a 256 -c -
chmod +x "$state/kind"
child=""
trap '[[ -z "$child" ]] || kill -TERM "$child" 2>/dev/null || true' EXIT
"$state/kind" create cluster --name ayni-runtime-qualification \
  --config "$root/dev/fixtures/runtime-preparation/cluster/kind.yaml" \
  --kubeconfig "$state/kubeconfig" &
child=$!
while kill -0 "$child" 2>/dev/null; do
  headroom || { kill -TERM "$child"; wait "$child" || true; echo "host headroom guard stopped creation" >&2; exit 1; }
  sleep 2
done
wait "$child"; child=""
helm pull oci://quay.io/cilium/charts/cilium --version 1.20.4 --destination "$state"
printf '%s\n' "505abff8385811df1a949776fc42e9f10697989a332acc86d6e8933efcb855ed  $state/cilium-1.20.4.tgz" | shasum -a 256 -c -
headroom || { echo "host headroom too low for Cilium" >&2; exit 1; }
python3 "$root/dev/scripts/runtime-preparation-preload.py" quay.io/cilium/cilium@sha256:20fe92e4c86641233a450098c158d2d29085f3419931ab6bf681d2485dedf16e
python3 "$root/dev/scripts/runtime-preparation-preload.py" quay.io/cilium/operator-generic@sha256:7a7a3489dc66ad67e420d285c8a8d2e169b0957ad15e120f593d27081bf38d82
helm install cilium "$state/cilium-1.20.4.tgz" --namespace kube-system \
  --kube-context kind-ayni-runtime-qualification --kubeconfig "$state/kubeconfig" \
  --values "$root/dev/fixtures/runtime-preparation/cluster/cilium.yaml" --wait --timeout 5m
kubectl --context kind-ayni-runtime-qualification --kubeconfig "$state/kubeconfig" wait \
  --for=condition=Ready node/ayni-runtime-qualification-control-plane --timeout=180s
printf 'context=kind-ayni-runtime-qualification\nkubeconfig=%s/kubeconfig\n' "$state"
echo 'Cluster provisioning is not qualification: run network, user-namespace and runtime acceptance gates.'
