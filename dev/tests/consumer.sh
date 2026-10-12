#!/usr/bin/env bash
# Share the disposable real-broker fixture; preserve accepted cleanup for this slice.
set -euo pipefail
export AYNI_CONSUMER_FIXTURE=true
exec bash "$(dirname -- "$0")/outbox.sh" "$@"
