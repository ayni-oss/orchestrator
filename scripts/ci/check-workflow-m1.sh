#!/usr/bin/env bash
set -euo pipefail
readonly root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
# One cumulative run: migrations/admission/watch evidence, canonical real-PG /
# real-JetStream / retained-ledger fault cases, transport permissions and limits.
export AYNI_M1_RUN_INTEGRATED=1
exec scripts/ci/check-workflow-postgres-feasibility.sh
