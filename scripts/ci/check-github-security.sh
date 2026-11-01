#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
readonly workflows="$repository_root/.github/workflows"

fail() {
  echo "GitHub security check: $*" >&2
  exit 1
}

[[ -d "$workflows" ]] || fail "workflow directory is missing"
if grep -RIn --include='*.yml' --include='*.yaml' 'pull_request_target' "$workflows"; then
  fail "pull_request_target is forbidden"
fi
if grep -En 'packages:[[:space:]]*write' "$workflows/pr-validation.yml"; then
  fail "pull-request validation must not receive package-write access"
fi

while IFS= read -r use; do
  action="${use##*uses: }"
  [[ "$action" == ./* ]] && continue
  if [[ "$action" =~ ^ayni-oss/ayni/\.github/actions/[a-z-]+@[0-9a-f]{40}[[:space:]]+#[[:space:]]shared[[:space:]]CI$ ]]; then
    continue
  fi
  if [[ ! "$action" =~ @[0-9a-f]{40}[[:space:]]+#[[:space:]]v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    fail "external Action requires an immutable SHA and exact release or shared CI comment: $use"
  fi
done < <(grep -RhE --include='*.yml' --include='*.yaml' '^[[:space:]]*(-[[:space:]]+)?uses:' "$workflows")

grep -Eq '^permissions: \{\}$' "$workflows/pr-validation.yml" \
  || fail "PR validation requires an empty workflow-level permission baseline"
grep -Eq 'persist-credentials: false' "$workflows/pr-validation.yml" \
  || fail "PR validation must disable persisted checkout credentials"

python3 - "$repository_root/scripts/ci/ayni-tool.json" <<'PY'
import json
import pathlib
import sys
import tomllib

release = json.loads(pathlib.Path(sys.argv[1]).read_text())
def exact_hex(value, length):
    return len(value) == length and all(c in "0123456789abcdef" for c in value)

if not exact_hex(release["source_revision"], 40):
    raise SystemExit("GitHub security check: Ayni source revision is not pinned")
repository = pathlib.Path(sys.argv[1]).resolve().parents[2]
for manifest in ("backend/servers/control-plane/Cargo.toml",
                 "backend/workers/runtime-preparation/Cargo.toml",
                 "backend/workers/runtime-verifier/Cargo.toml"):
    dependencies = tomllib.loads((repository / manifest).read_text())["dependencies"]
    for name in ("ayni-core", "ayni-environment"):
        dependency = dependencies[name]
        if dependency.get("git") != "https://github.com/ayni-oss/ayni.git" or dependency.get("rev") != release["source_revision"]:
            raise SystemExit(f"GitHub security check: {manifest} {name} differs from the admitted Ayni source")
if not exact_hex(release["linux_arm64_cli_sha256"], 64):
    raise SystemExit("GitHub security check: Ayni runtime CLI checksum is not pinned")
executor_prefix = "ghcr.io/ayni-oss/ayni-builder@sha256:"
executor = release["linux_arm64_executor_image"]
if not executor.startswith(executor_prefix) or not exact_hex(executor.removeprefix(executor_prefix), 64):
    raise SystemExit("GitHub security check: Ayni runtime executor is not pinned")
if not release["cli_version"]:
    raise SystemExit("GitHub security check: Ayni CLI version is not pinned")
for architecture in ("amd64", "arm64"):
    checksum = release[f"linux_{architecture}_archive_sha256"]
    if len(checksum) != 64 or any(c not in "0123456789abcdef" for c in checksum):
        raise SystemExit("GitHub security check: Ayni CLI checksum is not pinned")
PY
