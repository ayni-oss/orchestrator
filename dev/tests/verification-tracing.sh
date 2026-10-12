#!/usr/bin/env bash
# Exercise connected verification tracing, then the same acceptance with the
# Collector down. The existing harness owns isolated synthetic state and cleanup.
set -euo pipefail
root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
if [[ $# != 3 ]]; then
  echo 'usage: dev/tests/verification-tracing.sh CONTROL_PLANE_IMAGE PINNED_WORKER_IMAGE WEB_IMAGE' >&2
  exit 2
fi
VERIFICATION_TRACING_TEST=true "$root/dev/tests/verification-reporting.sh" "$@" events
VERIFICATION_TRACING_TEST=true VERIFICATION_COLLECTOR_OUTAGE=true "$root/dev/tests/verification-reporting.sh" "$@" events
