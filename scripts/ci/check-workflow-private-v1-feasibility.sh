#!/bin/sh
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"

command -v buf >/dev/null 2>&1 || {
  printf '%s\n' 'buf is required for the private-v1 cutover gate.' >&2
  exit 1
}

grep -qx 'workflow-m1-private-v1' proto/BASELINE
buf format --diff --exit-code proto
buf lint proto

python3 - <<'PY'
from pathlib import Path
import re

root = Path("proto/ayni/orchestrator/v1")
expected = {
    "projects.proto": {
        "ProjectService": ["CreateWorkspace", "CreateProject", "ListWorkspaces", "ListProjects"],
    },
    "project_conversations.proto": {
        "ProjectConversationService": [
            "CreateConversation",
            "SendMessage",
            "GetConversationSnapshot",
            "ListConversationHistory",
            "WatchConversation",
            "ListConversations",
        ],
    },
}
expected_files = set(expected) | {"errors.proto", "workflow.proto"}
actual_files = {path.name for path in root.glob("*.proto")}
if actual_files != expected_files:
    raise SystemExit(
        "unexpected private-v1 proto files: "
        f"expected={sorted(expected_files)!r} actual={sorted(actual_files)!r}"
    )

for filename, services in expected.items():
    source = (root / filename).read_text()
    actual: dict[str, list[str]] = {}
    for service, body in re.findall(r"service\s+(\w+)\s*\{(.*?)\n\}", source, re.S):
        actual[service] = re.findall(r"\brpc\s+(\w+)\s*\(", body)
    if actual != services:
        raise SystemExit(f"unexpected canonical RPC surface in {filename}: {actual!r}")

workflow = (root / "workflow.proto").read_text()
for symbol in (
    "CommandReceipt",
    "ConversationSnapshot",
    "ConversationHistoryRecord",
    "ConversationUpdate",
    "SchedulingState",
    "DispatchState",
    "ExecutionState",
    "PresentationState",
):
    if not re.search(rf"\b(message|enum)\s+{symbol}\b", workflow):
        raise SystemExit(f"canonical workflow symbol is missing: {symbol}")
PY

# Superseded handwritten private-v1 RPC consumers must not survive the reset.
legacy='GetWorkspace|UpdateWorkspace|ArchiveWorkspace|WatchProjects|ListProjectSummaries|GetProject|UpdateProject|ArchiveProject|UpdateProjectConversationSettings|CancelProjectAgentRun|GetProjectConversationContext|ListProjectConversationHistory|WatchProjectConversation|CreateProjectConversation|ListProjectConversations|GetProjectConversation|SendProjectMessage|DecideProjectPlan|DecideProjectProposal'
if rg -n --glob '!**/src/generated/**' --glob '!docs/**' --glob '!scripts/ci/check-workflow-private-v1-feasibility.sh' \
  "$legacy" backend clients packages apps dev; then
  printf '%s\n' 'superseded handwritten private-v1 consumer remains' >&2
  exit 1
fi

# Workflow services are wired only to the trusted loopback listener. There is
# no public/custom workflow listener in the replacement control plane or chart.
grep -q 'DEFAULT_GRPC_BIND_ADDRESS.*' backend/servers/control-plane/src/configuration.rs
grep -q '"127.0.0.1:50051"' backend/servers/control-plane/src/configuration.rs
grep -q 'if !address.ip().is_loopback()' backend/servers/control-plane/src/workflow.rs
if rg -n '50052|custom.?client' backend/servers/control-plane/src deploy/charts/ayni-orchestrator/templates dev/charts/ayni-orchestrator-local/templates; then
  printf '%s\n' 'custom-client workflow exposure remains in the current runtime' >&2
  exit 1
fi

printf '%s\n' 'private-v1-cutover: canonical-surface=pass stale-consumers=absent trusted-loopback=pass custom-listener=absent'
