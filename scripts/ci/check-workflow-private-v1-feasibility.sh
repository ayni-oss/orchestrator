#!/bin/sh
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
candidate="$root/dev/feasibility/private-v1"
candidate_proto="$candidate/proto"
plugin="$root/clients/typescript/node_modules/.bin/protoc-gen-ts_proto"

command -v buf >/dev/null 2>&1 || {
  printf '%s\n' 'buf is required for the private-v1 feasibility gate.' >&2
  exit 1
}
command -v rustfmt >/dev/null 2>&1 || {
  printf '%s\n' 'rustfmt is required for the private-v1 feasibility gate.' >&2
  exit 1
}
test -x "$plugin" || {
  printf '%s\n' 'TypeScript protocol tools are absent; run pnpm install --frozen-lockfile.' >&2
  exit 1
}

# The candidate must remain isolated until the coordinated authority switch.
test ! -e "$root/proto/ayni/orchestrator/v1/workflow.proto"
if rg -q 'rpc (CreateConversation|SendMessage|GetConversationSnapshot|WatchConversation)' \
  "$root/proto/ayni/orchestrator/v1"; then
  printf '%s\n' 'candidate workflow RPCs leaked into the canonical production schema' >&2
  exit 1
fi

rustfmt --edition 2024 --check "$candidate/listener_model.rs"
buf format --diff --exit-code "$candidate_proto"
buf lint "$candidate_proto"

temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT HUP INT TERM

overlay="$temporary/overlay"
mkdir -p "$overlay/ayni/orchestrator/v1"
cp -R "$root/proto/." "$overlay/"
cp "$candidate_proto/ayni/orchestrator/v1/projects.proto" "$overlay/ayni/orchestrator/v1/projects.proto"
cp "$candidate_proto/ayni/orchestrator/v1/project_conversations.proto" "$overlay/ayni/orchestrator/v1/project_conversations.proto"
cp "$candidate_proto/ayni/orchestrator/v1/workflow.proto" "$overlay/ayni/orchestrator/v1/workflow.proto"

buf format --diff --exit-code "$overlay"
buf lint "$overlay"

# The old private-v1 baseline must reject the intentional replacement.
if buf breaking "$overlay" --against "$root/proto" >"$temporary/old-breaking.log" 2>&1; then
  printf '%s\n' 'old private-v1 baseline unexpectedly accepted the M1 reset' >&2
  exit 1
fi
grep -q 'Previously present.*was deleted' "$temporary/old-breaking.log"

# Once the one-time baseline is refreshed, the exact candidate must pass FILE.
refreshed="$temporary/refreshed"
cp -R "$overlay" "$refreshed"
buf breaking "$overlay" --against "$refreshed"

# Normal FILE enforcement must resume after the reset. Removing one RPC must fail.
mutated="$temporary/mutated"
cp -R "$overlay" "$mutated"
python3 - "$mutated/ayni/orchestrator/v1/project_conversations.proto" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text()
line = "  rpc SendMessage(SendMessageRequest) returns (SendMessageResponse);\n"
if source.count(line) != 1:
    raise SystemExit("expected exactly one SendMessage RPC")
path.write_text(source.replace(line, ""))
PY
if buf breaking "$mutated" --against "$refreshed" >"$temporary/post-reset-breaking.log" 2>&1; then
  printf '%s\n' 'post-reset FILE policy did not detect a removed RPC' >&2
  exit 1
fi
grep -q 'RPC.*SendMessage.*was deleted' "$temporary/post-reset-breaking.log"

# Generate the complete overlaid module twice; output must be deterministic.
cat >"$temporary/buf.gen.yaml" <<EOF
version: v2
clean: true
plugins:
  - local:
      - $plugin
    out: generated
    strategy: all
    opt:
      - env=node
      - esModuleInterop=true
      - exportCommonSymbols=false
      - forceLong=string
      - oneof=unions
      - outputIndex=true
      - outputServices=grpc-js
      - useExactTypes=false
      - useOptionals=messages
EOF
mkdir "$temporary/first" "$temporary/second"
(cd "$temporary/first" && buf generate "$overlay" --template "$temporary/buf.gen.yaml")
(cd "$temporary/second" && buf generate "$overlay" --template "$temporary/buf.gen.yaml")
diff -ru "$temporary/first/generated" "$temporary/second/generated"

# The checked inventory detects handwritten consumer drift without counting generated output.
rg -l --glob '!clients/typescript/src/generated/**' \
  --glob '!dev/feasibility/private-v1/**' \
  'ProjectConversationService|ProjectService|CreateWorkspaceRequest|CreateProjectRequest|ProjectConversation|SendProjectMessage|WatchProjectConversation' \
  "$root/backend" "$root/packages" "$root/apps" "$root/dev" \
  | sed "s|^$root/||" | LC_ALL=C sort >"$temporary/consumer-inventory.txt"
diff -u "$candidate/consumer-inventory.txt" "$temporary/consumer-inventory.txt"

# Compile and execute the positive listener model.
rustc --edition=2024 --test "$candidate/listener_model.rs" -o "$temporary/listener-model-test"
"$temporary/listener-model-test"

# SQLite cannot satisfy the sealed workflow-owner bound.
cat >"$temporary/sqlite-owner.rs" <<EOF
#[path = "$candidate/listener_model.rs"]
mod listener_model;
use listener_model::{SqliteWorkflowOwner, TrustedOperatorListener};
fn main() {
    let _ = TrustedOperatorListener::new(SqliteWorkflowOwner);
}
EOF
if rustc --edition=2024 "$temporary/sqlite-owner.rs" -o "$temporary/sqlite-owner" \
  >"$temporary/sqlite-owner.log" 2>&1; then
  printf '%s\n' 'SQLite unexpectedly satisfied the replacement workflow-owner bound' >&2
  exit 1
fi
grep -q 'SqliteWorkflowOwner.*WorkflowOwner.*not satisfied' "$temporary/sqlite-owner.log"

# The custom-client listener has no workflow-registration operation.
cat >"$temporary/custom-listener.rs" <<EOF
#[path = "$candidate/listener_model.rs"]
mod listener_model;
use listener_model::{CustomClientListener, PostgresWorkflowOwner};
fn main() {
    let listener = CustomClientListener::new();
    listener.register_workflow(PostgresWorkflowOwner);
}
EOF
if rustc --edition=2024 "$temporary/custom-listener.rs" -o "$temporary/custom-listener" \
  >"$temporary/custom-listener.log" 2>&1; then
  printf '%s\n' 'custom-client listener unexpectedly accepted workflow registration' >&2
  exit 1
fi
grep -q 'no method named.*register_workflow' "$temporary/custom-listener.log"

printf '%s\n' \
  'private-v1-feasibility: schema=pass old-baseline-break=pass refreshed-baseline=pass post-reset-file-policy=pass generation=deterministic inventory=pass trusted-postgres-owner=pass custom-listener-omission=pass'
