#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
readonly scratch="$(mktemp -d)"
trap 'rm -r -- "$scratch"' EXIT

rustc "$repository_root/examples/retry-backoff/main.rs" -o "$scratch/retry-backoff"

for case in '1 2' '2 4' '3 8' '6 60'; do
  read -r attempt expected <<<"$case"
  actual="$("$scratch/retry-backoff" "$attempt")"
  if [[ "$actual" != "$expected" ]]; then
    echo "attempt $attempt: expected $expected seconds, got $actual" >&2
    exit 1
  fi
done

echo "retry-backoff formula matches all four empirical cases"
