#!/usr/bin/env bash
set -euo pipefail

readonly dev_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$dev_directory/lib/common.sh"
source "$dev_directory/lib/images.sh"
readonly scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
readonly calls="$scratch/calls"
readonly mock_image_id=sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
readonly expected_revision="$(git -C "$DEV_REPOSITORY_ROOT" rev-parse --short=12 HEAD)"
failure=""

# No daemon is needed to exercise command-substitution failure propagation.
docker() {
  local step
  case "$1 $2" in
    'build --file') step=build ;;
    'image inspect')
      if [[ "$5" == '{{.Id}}' ]]; then step=inspect-id; else step=inspect-final; fi
      ;;
    'image tag') step=tag ;;
    'image remove') step=remove ;;
    *) echo "unexpected Docker command: $*" >&2; return 2 ;;
  esac
  printf '%s\n' "$step" >> "$calls"
  if [[ "$failure" == "$step" ]]; then
    echo "simulated $step failure" >&2
    return 42
  fi
  if [[ "$step" == inspect-id ]]; then
    case "$failure" in
      empty-id) return 0 ;;
      invalid-id) printf 'invalid\n' ;;
      *) printf '%s\n' "$mock_image_id" ;;
    esac
  fi
}

for failure in build inspect-id empty-id invalid-id tag remove inspect-final; do
  : > "$calls"
  result=""
  # An if condition also suppresses errexit: the helper must explicitly fail.
  if result="$(dev_build_image worker 2>"$scratch/error")"; then
    echo "image build incorrectly succeeded after $failure" >&2
    exit 1
  else
    status=$?
  fi
  [[ -z "$result" ]] || { echo "failed build emitted an image reference" >&2; exit 1; }
  if [[ "$failure" == empty-id || "$failure" == invalid-id ]]; then
    [[ "$status" -eq 1 ]]
    last_step=inspect-id
  else
    [[ "$status" -eq 42 ]]
    last_step="$failure"
    grep -Fq "simulated $failure failure" "$scratch/error"
  fi
  [[ "$(tail -n 1 "$calls")" == "$last_step" ]]
done

failure=""
: > "$calls"
result="$(dev_build_image worker)"
[[ "$result" == "local/ayni-orchestrator-worker:git-${expected_revision}-aaaaaaaaaaaa" ]]
[[ "$(cat "$calls")" == $'build\ninspect-id\ntag\nremove\ninspect-final' ]]
: > "$calls"
result="$(dev_build_image worker local/ayni-orchestrator-worker:explicit)"
[[ "$result" == local/ayni-orchestrator-worker:explicit ]]
[[ "$(cat "$calls")" == $'build\ninspect-final' ]]
echo 'image build failure propagation and immutable references passed'
