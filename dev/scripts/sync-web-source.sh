#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$repository_root/dev/lib/common.sh"
readonly namespace="$(dev_namespace)"

# A fresh Tilt session must copy existing edits into the development image.
# Only UI source is synchronized; dependencies and deployment inputs stay fixed.
COPYFILE_DISABLE=1 tar -C "$repository_root" -cf - \
  apps/web/src apps/web/static packages/bff/src clients/typescript/src \
  tsconfig.base.json \
  | kubectl --context docker-desktop --namespace "$namespace" \
      exec -i deployment/control-plane -c web -- tar -xf - -C /opt/source
