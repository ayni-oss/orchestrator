#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$repository_root/dev/lib/common.sh"
readonly namespace="$(dev_namespace)"

# Bring an existing development image up to the current checkout before Tilt
# attaches. Never copy build output, dependencies or deployment credentials.
COPYFILE_DISABLE=1 tar -C "$repository_root" -cf - \
  backend proto Cargo.toml Cargo.lock rust-toolchain.toml \
  dev/scripts/restart-control-plane.sh \
  | kubectl --context docker-desktop --namespace "$namespace" \
      exec -i deployment/control-plane -c control-plane -- tar -xf - -C /opt/source
kubectl --context docker-desktop --namespace "$namespace" \
  exec deployment/control-plane -c control-plane -- \
  sh -c 'cd /opt/source && cargo build --locked --jobs 1 --package ayni-orchestrator-control-plane --bins && sh dev/scripts/restart-control-plane.sh'
