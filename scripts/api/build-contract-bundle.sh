#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
if [[ $# -ne 1 ]]; then
  echo "usage: $0 OUTPUT_DIRECTORY" >&2
  exit 2
fi
if ! command -v buf >/dev/null 2>&1; then
  echo "buf is required to build the canonical descriptor set" >&2
  exit 1
fi
if [[ -n "$(git -C "$repository_root" status --porcelain --untracked-files=all -- proto)" ]]; then
  echo "proto sources and contract documents must be committed before bundling" >&2
  exit 1
fi

readonly output_directory="$1"
readonly revision="$(git -C "$repository_root" rev-parse HEAD)"
readonly bundle_name="ayni-orchestrator-v1-${revision:0:12}"
readonly archive="$output_directory/$bundle_name.tar.gz"
readonly archive_checksum="$archive.sha256"
readonly work_directory="$(mktemp -d "${TMPDIR:-/tmp}/orchestrator-contract.XXXXXX")"
cleanup() { rm -rf "$work_directory"; }
trap cleanup EXIT

mkdir -p "$output_directory" "$work_directory/$bundle_name"
if [[ -e "$archive" || -e "$archive_checksum" ]]; then
  echo "refusing to overwrite an existing contract bundle for revision $revision" >&2
  exit 1
fi
git -C "$repository_root" archive HEAD \
  proto/ayni proto/README.md proto/RELEASES.md proto/buf.yaml \
  | tar -x -C "$work_directory/$bundle_name"
buf build "$work_directory/$bundle_name/proto" --as-file-descriptor-set \
  --output "$work_directory/$bundle_name/ayni.orchestrator.v1.pb"
printf 'package=ayni.orchestrator.v1\nrevision=%s\ncompatibility=additive-only\n' "$revision" \
  >"$work_directory/$bundle_name/VERSION"
(
  cd "$work_directory/$bundle_name"
  find . -type f ! -name SHA256SUMS -print0 \
    | sort -z \
    | xargs -0 shasum -a 256 \
    >SHA256SUMS
)
tar -C "$work_directory" -czf "$archive" "$bundle_name"
(
  cd "$output_directory"
  shasum -a 256 "$bundle_name.tar.gz" >"$bundle_name.tar.gz.sha256"
)
printf '%s\n' "$archive"
