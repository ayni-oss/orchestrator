#!/bin/sh
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"

if [ ! -x clients/typescript/node_modules/.bin/protoc-gen-ts_proto ]; then
  printf '%s\n' 'TypeScript protocol tools are absent; run pnpm install --frozen-lockfile.' >&2
  exit 1
fi

buf format --diff --exit-code proto
buf lint proto

if [ "$#" -gt 1 ]; then
  printf '%s\n' 'usage: scripts/ci/check-proto.sh [BASE_PROTO_DIRECTORY]' >&2
  exit 2
fi
if [ "$#" -eq 1 ]; then
  test -f "$1/buf.yaml" || {
    printf 'base protobuf module is absent: %s\n' "$1" >&2
    exit 1
  }
  buf breaking proto --against "$1"
fi

temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
cp -R clients/typescript/src/generated "$temporary/generated"
buf generate
diff -ru "$temporary/generated" clients/typescript/src/generated
pnpm --filter @ayni-orchestrator/grpc-bindings check
