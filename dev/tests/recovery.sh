#!/usr/bin/env bash
# Real ACK faults and bounded recovery against production images in one namespace.
set -euo pipefail
export AYNI_CONSUMER_FIXTURE=true AYNI_RECOVERY_FIXTURE=true
exec bash "$(dirname -- "$0")/outbox.sh" "$@"
