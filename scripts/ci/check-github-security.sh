#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
readonly workflows="$repository_root/.github/workflows"
readonly app_setup="$repository_root/apps/web/src/routes/setup/github/+page.svelte"

fail() {
  echo "GitHub security check: $*" >&2
  exit 1
}

[[ -d "$workflows" ]] || fail "workflow directory is missing"
[[ -f "$app_setup" ]] || fail "GitHub App setup page is missing"

if rg -n --glob '*.yml' --glob '*.yaml' 'pull_request_target' "$workflows"; then
  fail "pull_request_target is forbidden"
fi
if rg -n 'packages:\s*write' "$workflows/pr-validation.yml"; then
  fail "pull-request validation must not receive package-write access"
fi

while IFS= read -r use; do
  action="${use#*uses: }"
  if [[ ! "$action" =~ @[0-9a-f]{40}([[:space:]]*#[[:space:]]*v[0-9][^[:space:]]*)?$ ]]; then
    fail "third-party Action is not pinned to a full SHA with a release comment: $use"
  fi
done < <(rg --no-filename --glob '*.yml' --glob '*.yaml' '^\s*uses:' "$workflows")

for permission in actions checks contents issues metadata packages pull_requests workflows; do
  rg -q "${permission}: \"(read|write)\"" "$app_setup" \
    || fail "GitHub App manifest is missing $permission permission"
done
for read_only in actions checks packages; do
  rg -q "${read_only}: \"read\"" "$app_setup" \
    || fail "GitHub App manifest must keep $read_only read-only"
done
for write_permission in contents issues pull_requests workflows; do
  rg -q "${write_permission}: \"write\"" "$app_setup" \
    || fail "GitHub App manifest must grant $write_permission write access"
done

rg -q '^permissions: \{\}$' "$workflows/pr-validation.yml" \
  || fail "PR validation requires an empty workflow-level permission baseline"
rg -q 'persist-credentials: false' "$workflows/pr-validation.yml" \
  || fail "PR validation must disable persisted checkout credentials"
