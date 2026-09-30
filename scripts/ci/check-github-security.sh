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

for permission in actions checks contents issues metadata packages pull_requests workflows; do
  grep -Eq "${permission}: \"(read|write)\"" "$app_setup" \
    || fail "GitHub App manifest is missing $permission permission"
done
for read_only in actions checks packages; do
  grep -Eq "${read_only}: \"read\"" "$app_setup" \
    || fail "GitHub App manifest must keep $read_only read-only"
done
for write_permission in contents issues pull_requests workflows; do
  grep -Eq "${write_permission}: \"write\"" "$app_setup" \
    || fail "GitHub App manifest must grant $write_permission write access"
done

grep -Eq '^permissions: \{\}$' "$workflows/pr-validation.yml" \
  || fail "PR validation requires an empty workflow-level permission baseline"
grep -Eq 'persist-credentials: false' "$workflows/pr-validation.yml" \
  || fail "PR validation must disable persisted checkout credentials"
