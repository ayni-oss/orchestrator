#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
readonly workflows="$repository_root/.github/workflows"
readonly app_setup="$repository_root/apps/web/src/lib/components/github-connection-manager.svelte"

fail() {
  echo "GitHub security check: $*" >&2
  exit 1
}

[[ -d "$workflows" ]] || fail "workflow directory is missing"
[[ -f "$app_setup" ]] || fail "GitHub App manifest component is missing"

if grep -RIn --include='*.yml' --include='*.yaml' 'pull_request_target' "$workflows"; then
  fail "pull_request_target is forbidden"
fi
if grep -En 'packages:[[:space:]]*write' "$workflows/pr-validation.yml"; then
  fail "pull-request validation must not receive package-write access"
fi

while IFS= read -r use; do
  action="${use#*uses: }"
  if [[ ! "$action" =~ @[0-9a-f]{40}([[:space:]]*#[[:space:]]*v[0-9][^[:space:]]*)?$ ]]; then
    fail "third-party Action is not pinned to a full SHA with a release comment: $use"
  fi
done < <(grep -RhE --include='*.yml' --include='*.yaml' '^[[:space:]]*uses:' "$workflows")

python3 - "$app_setup" <<'PYTHON'
import json
import pathlib
import re
import sys

source = pathlib.Path(sys.argv[1]).read_text()
blocks = re.findall(r"default_permissions:\s*\{([^}]+)\}", source, re.S)
if len(blocks) != 1:
    sys.exit("GitHub security check: expected one authoritative App permission manifest")
body = re.sub(r"(\w+):", r'"\1":', blocks[0])
body = re.sub(r",\s*$", "", body)
try:
    permissions = json.loads("{" + body + "}")
except json.JSONDecodeError:
    sys.exit("GitHub security check: App permission manifest could not be checked")
expected = {"contents": "write", "metadata": "read", "pull_requests": "write"}
if permissions != expected:
    sys.exit("GitHub security check: App permissions must be exactly Contents/Pull requests write and Metadata read")
PYTHON

grep -Eq '^permissions: \{\}$' "$workflows/pr-validation.yml" \
  || fail "PR validation requires an empty workflow-level permission baseline"
grep -Eq 'persist-credentials: false' "$workflows/pr-validation.yml" \
  || fail "PR validation must disable persisted checkout credentials"

# CI tool selection is independent of repository runtime compatibility.
python3 - "$repository_root" <<'PYTHON'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
release = json.loads((root / "scripts/ci/ayni-tool.json").read_text())
assert release["cli_version"] and len(release["linux_amd64_archive_sha256"]) == 64
worker = (root / "deploy/images/worker/Dockerfile").read_text()
assert "AYNI_EXECUTOR_IMAGE" not in worker and "ARG AYNI_VERSION" not in worker, "worker must consume repository-selected official builders"
assert "mise_sha=" in worker and "sha256sum --check" in worker
for architecture in ("amd64", "arm64"):
    checksum = release[f"linux_{architecture}_archive_sha256"]
    assert len(checksum) == 64 and all(c in "0123456789abcdef" for c in checksum), "CI CLI checksum must be pinned"
PYTHON
