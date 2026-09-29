#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
if [[ $# -ne 0 ]]; then
  echo "usage: $0" >&2
  exit 2
fi
context="$($repository_root/dev/scripts/require-docker-desktop-context.sh)"
readonly context
namespace="$("$repository_root/dev/scripts/resolve-namespace.sh")"
readonly namespace
readonly grpc_port="${AYNI_ORCHESTRATOR_CUSTOM_GRPC_LOCAL_PORT:-15052}"
readonly ca_file="${ORCHESTRATOR_CUSTOM_GRPC_CA_FILE:?ORCHESTRATOR_CUSTOM_GRPC_CA_FILE is required}"
readonly token_file="${ORCHESTRATOR_CUSTOM_GRPC_TOKEN_FILE:?ORCHESTRATOR_CUSTOM_GRPC_TOKEN_FILE is required}"
readonly tls_name="${ORCHESTRATOR_CUSTOM_GRPC_TLS_NAME:?ORCHESTRATOR_CUSTOM_GRPC_TLS_NAME is required}"
readonly descriptor="${ORCHESTRATOR_CUSTOM_GRPC_DESCRIPTOR:?ORCHESTRATOR_CUSTOM_GRPC_DESCRIPTOR is required}"
readonly forward_log="$(mktemp /tmp/ayni-orchestrator-custom-grpc-forward.XXXXXX)"

if ! command -v grpcurl >/dev/null 2>&1; then
  echo "grpcurl is required for the protected gRPC smoke check" >&2
  exit 1
fi
for file in "$ca_file" "$token_file" "$descriptor"; do
  if [[ ! -r "$file" ]]; then
    echo "required custom-client file is not readable: $file" >&2
    exit 1
  fi
done

kubectl --context "$context" --namespace "$namespace" port-forward --address 127.0.0.1 \
  service/control-plane "$grpc_port:50052" >"$forward_log" 2>&1 &
readonly forward_pid="$!"
cleanup() {
  kill "$forward_pid" 2>/dev/null || true
  wait "$forward_pid" 2>/dev/null || true
  rm -f "$forward_log"
}
trap cleanup EXIT

check_once() {
  local missing models token
  if missing="$(grpcurl -max-time 10 -cacert "$ca_file" -authority "$tls_name" \
      -protoset "$descriptor" -d '{}' "127.0.0.1:$grpc_port" \
      ayni.orchestrator.v1.ProviderService/ListProviderModels 2>&1)"; then
    echo "protected listener accepted a request without a credential" >&2
    return 1
  fi
  grep -q 'Code: Unauthenticated' <<<"$missing" || return 1

  token="$(tr -d '\r\n' <"$token_file")"
  [[ ${#token} -ge 32 && ${#token} -le 512 ]] || return 1
  models="$(ORCHESTRATOR_TOKEN="$token" grpcurl -expand-headers -max-time 10 \
    -cacert "$ca_file" -authority "$tls_name" -protoset "$descriptor" \
    -rpc-header 'authorization: Bearer ${ORCHESTRATOR_TOKEN}' \
    -rpc-header 'x-request-id: custom-client-supported-path-check' \
    -d '{}' "127.0.0.1:$grpc_port" \
    ayni.orchestrator.v1.ProviderService/ListProviderModels)" || return 1
  grep -q '"provider": "openai"' <<<"$models" || return 1
  printf '%s\n' 'protected custom-client gRPC check passed'
}

for _ in {1..30}; do
  if check_once; then
    exit 0
  fi
  sleep 1
done

cat "$forward_log" >&2
exit 1
