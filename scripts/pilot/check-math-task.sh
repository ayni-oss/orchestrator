#!/usr/bin/env bash
set -euo pipefail

readonly task="${1:?usage: scripts/pilot/check-math-task.sh TASK CHECKOUT EXPECTED_COMMIT EXPECTED_LOCK_FINGERPRINT EXPECTED_START_COMMIT}"
readonly checkout="${2:?usage: scripts/pilot/check-math-task.sh TASK CHECKOUT EXPECTED_COMMIT EXPECTED_LOCK_FINGERPRINT EXPECTED_START_COMMIT}"
readonly expected_commit="${3:?usage: scripts/pilot/check-math-task.sh TASK CHECKOUT EXPECTED_COMMIT EXPECTED_LOCK_FINGERPRINT EXPECTED_START_COMMIT}"
readonly expected_lock="${4:?usage: scripts/pilot/check-math-task.sh TASK CHECKOUT EXPECTED_COMMIT EXPECTED_LOCK_FINGERPRINT EXPECTED_START_COMMIT}"
readonly expected_start="${5:?usage: scripts/pilot/check-math-task.sh TASK CHECKOUT EXPECTED_COMMIT EXPECTED_LOCK_FINGERPRINT EXPECTED_START_COMMIT}"
readonly manifest="$checkout/Cargo.toml"
readonly lock="$checkout/.ayni.lock"
readonly command_timeout_seconds=60
readonly invocation_timeout_seconds=90

test -f "$manifest" || { echo "sandbox Cargo.toml not found: $manifest" >&2; exit 2; }
test -f "$lock" || { echo "sandbox Ayni lock not found: $lock" >&2; exit 2; }
test -f "$checkout/.ayni/environment/build.json" || {
  echo "sandbox environment build record is missing; run 'ayni env build' first" >&2
  exit 2
}
grep -Fxq 'name = "math-validation"' "$manifest" || {
  echo "checkout is not the math-validation sandbox" >&2
  exit 2
}

readonly actual_commit="$(git -C "$checkout" rev-parse HEAD)"
[[ "$actual_commit" == "$expected_commit" ]] || {
  echo "checkout commit $actual_commit does not match $expected_commit" >&2
  exit 2
}
git -C "$checkout" cat-file -e "$expected_start^{commit}" 2>/dev/null || {
  echo "expected start commit is absent: $expected_start" >&2
  exit 2
}
git -C "$checkout" merge-base --is-ancestor "$expected_start" "$expected_commit" || {
  echo "candidate $expected_commit does not descend from $expected_start" >&2
  exit 2
}
[[ -z "$(git -C "$checkout" status --porcelain --untracked-files=all)" ]] || {
  echo "checkout must be clean before validation" >&2
  exit 2
}
readonly origin="$(git -C "$checkout" remote get-url origin)"
case "$origin" in
  git@github.com:ayni-oss/sandbox.git|https://github.com/ayni-oss/sandbox|https://github.com/ayni-oss/sandbox.git) ;;
  *) echo "unexpected sandbox origin: $origin" >&2; exit 2 ;;
esac
readonly actual_lock="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["fingerprint"])' "$lock")"
[[ "$actual_lock" == "$expected_lock" ]] || {
  echo "Ayni lock $actual_lock does not match $expected_lock" >&2
  exit 2
}

snapshot_parent="$(mktemp -d)"
readonly snapshot_parent
readonly validation_checkout="$snapshot_parent/repository"
cleanup() {
  chmod -R u+w "$snapshot_parent" 2>/dev/null || true
  rm -rf "$snapshot_parent"
}
trap cleanup EXIT

git clone --quiet --no-hardlinks "$checkout" "$validation_checkout"
git -C "$validation_checkout" checkout --quiet --detach "$expected_commit"
cp -R "$checkout/.ayni" "$validation_checkout/.ayni"
chmod -R a-w "$validation_checkout"
chmod -R u+w "$validation_checkout/.ayni"

assert_snapshot_unchanged() {
  [[ "$(git -C "$validation_checkout" rev-parse HEAD)" == "$expected_commit" ]] || {
    echo "candidate execution changed the snapshot commit" >&2
    return 125
  }
  [[ -z "$(git -C "$validation_checkout" status --porcelain --untracked-files=all)" ]] || {
    echo "candidate execution changed the immutable snapshot" >&2
    return 125
  }
  [[ "$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["fingerprint"])' "$validation_checkout/.ayni.lock")" == "$expected_lock" ]] || {
    echo "candidate execution changed the snapshot Ayni lock" >&2
    return 125
  }
}

run_with_timeout() {
  python3 - "$invocation_timeout_seconds" "$@" <<'PY'
import os
import signal
import subprocess
import sys

timeout = int(sys.argv[1])
process = subprocess.Popen(sys.argv[2:], start_new_session=True)
try:
    raise SystemExit(process.wait(timeout=timeout))
except subprocess.TimeoutExpired:
    os.killpg(process.pid, signal.SIGTERM)
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait()
    raise SystemExit(124)
PY
}

run_candidate() {
  local status
  if run_with_timeout \
    ayni env run --repo-root "$validation_checkout" -- \
    timeout --signal=TERM --kill-after=5 "$command_timeout_seconds" \
    env CARGO_TARGET_DIR=/tmp/math-validation-target \
    cargo run --locked --quiet -- "$@"; then
    status=0
  else
    status=$?
  fi
  assert_snapshot_unchanged || return 125
  return "$status"
}

expect_output() {
  local expected="$1"
  shift
  local actual
  actual="$(run_candidate "$@")" || {
    echo "command failed or exceeded ${command_timeout_seconds}s: $*" >&2
    exit 1
  }
  if [[ "$actual" != "$expected" ]]; then
    echo "command '$*' produced '$actual', expected '$expected'" >&2
    exit 1
  fi
}

expect_failure() {
  local status
  set +e
  run_candidate "$@" >/dev/null 2>&1
  status=$?
  set -e
  if [[ "$status" -eq 0 ]]; then
    echo "command unexpectedly succeeded: $*" >&2
    exit 1
  fi
  if [[ "$status" -eq 124 || "$status" -eq 125 || "$status" -eq 137 ]]; then
    echo "invalid-input command timed out or violated candidate integrity: $*" >&2
    exit 1
  fi
}

case "$task" in
  gcd)
    expect_output 6 gcd 54 24
    expect_output 9 gcd 0 9
    expect_output 0 gcd 0 0
    expect_output 1 gcd 18446744073709551615 1
    expect_failure gcd 1
    expect_failure gcd 1 2 3
    expect_failure gcd one 2
    expect_failure gcd 18446744073709551616 2
    ;;
  lcm)
    expect_output 42 lcm 21 6
    expect_output 0 lcm 0 9
    expect_output 36 lcm 12 18
    expect_output 18446744073709551615 lcm 18446744073709551615 18446744073709551615
    expect_failure lcm 18446744073709551615 2
    expect_failure lcm 1
    expect_failure lcm 1 2 3
    expect_failure lcm one 2
    expect_failure lcm 18446744073709551616 2
    ;;
  is-prime)
    expect_output false is-prime 0
    expect_output false is-prime 1
    expect_output true is-prime 2
    expect_output false is-prime 9
    expect_output false is-prime 49
    expect_output true is-prime 97
    expect_output false is-prime 100
    expect_failure is-prime
    expect_failure is-prime 2 extra
    expect_failure is-prime prime
    expect_failure is-prime -1
    expect_failure is-prime 18446744073709551616
    ;;
  integer-sqrt)
    expect_output 0 integer-sqrt 0
    expect_output 3 integer-sqrt 15
    expect_output 4 integer-sqrt 16
    expect_output 4294967295 integer-sqrt 18446744073709551615
    expect_failure integer-sqrt
    expect_failure integer-sqrt 4 extra
    expect_failure integer-sqrt root
    expect_failure integer-sqrt -1
    expect_failure integer-sqrt 18446744073709551616
    ;;
  checked-mean)
    expect_output 7 checked-mean 7
    expect_output 4 checked-mean 2 4 6
    expect_output -4 checked-mean -5 -4
    expect_output 9223372036854775807 checked-mean 9223372036854775807 9223372036854775807
    expect_output 0 checked-mean 9223372036854775807 -9223372036854775808
    expect_failure checked-mean
    expect_failure checked-mean one
    expect_failure checked-mean 9223372036854775808
    expect_failure checked-mean -9223372036854775809
    ;;
  *)
    echo "unsupported task: $task" >&2
    exit 2
    ;;
esac

readonly quality_output="$snapshot_parent/quality.json"
if ! (cd "$validation_checkout" && run_with_timeout ayni check --output json >"$quality_output"); then
  assert_snapshot_unchanged || exit 125
  echo "committed Ayni quality contract failed" >&2
  exit 1
fi
assert_snapshot_unchanged || exit 125

echo "task=$task commit=$actual_commit lock=$actual_lock acceptance=passed"
