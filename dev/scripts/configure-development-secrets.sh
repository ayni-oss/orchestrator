#!/usr/bin/env bash
set -euo pipefail

readonly dev_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$dev_directory/lib/common.sh"
readonly repository_root="$DEV_REPOSITORY_ROOT"
namespace="$(dev_namespace)"
readonly namespace

usage() {
  echo "usage: $0 [--env-file PATH]" >&2
}

env_file="$repository_root/dev/.env"
if [[ $# -eq 2 && "$1" == "--env-file" ]]; then
  env_file="$2"
elif [[ $# -ne 0 ]]; then
  usage
  exit 2
fi

if [[ ! -f "$env_file" ]]; then
  echo "environment file does not exist: $env_file" >&2
  exit 1
fi
if env_file_mode="$(stat -f '%Lp' "$env_file" 2>/dev/null)"; then
  :
else
  env_file_mode="$(stat -c '%a' "$env_file")"
fi
if (( (8#$env_file_mode & 8#077) != 0 )); then
  echo "environment file must not be readable or writable by group or others: $env_file" >&2
  echo "run: chmod 600 '$env_file'" >&2
  exit 1
fi

github_app_id=""
github_app_installation_id=""
github_app_private_key_b64=""
openai_api_key=""
while IFS= read -r line || [[ -n "$line" ]]; do
  [[ -z "$line" || "$line" == \#* ]] && continue
  if [[ ! "$line" =~ ^([A-Z0-9_]+)=(.*)$ ]]; then
    echo "invalid environment-file entry" >&2
    exit 1
  fi
  key="${BASH_REMATCH[1]}"
  value="${BASH_REMATCH[2]}"
  case "$key" in
    GITHUB_APP_ID) github_app_id="$value" ;;
    GITHUB_APP_INSTALLATION_ID) github_app_installation_id="$value" ;;
    GITHUB_APP_PRIVATE_KEY_B64) github_app_private_key_b64="$value" ;;
    OPENAI_API_KEY) openai_api_key="$value" ;;
    *)
      echo "unsupported environment-file key: $key" >&2
      exit 1
      ;;
  esac
done < "$env_file"

for key in GITHUB_APP_ID GITHUB_APP_PRIVATE_KEY_B64 OPENAI_API_KEY; do
  case "$key" in
    GITHUB_APP_ID) value="$github_app_id" ;;
    GITHUB_APP_PRIVATE_KEY_B64) value="$github_app_private_key_b64" ;;
    OPENAI_API_KEY) value="$openai_api_key" ;;
  esac
  if [[ -z "$value" ]]; then
    echo "missing required environment-file key: $key" >&2
    exit 1
  fi
done
if [[ ! "$github_app_id" =~ ^[1-9][0-9]*$ ]]; then
  echo "GitHub App ID must be a positive decimal number" >&2
  exit 1
fi
if [[ -n "$github_app_installation_id" \
  && ! "$github_app_installation_id" =~ ^[1-9][0-9]*$ ]]; then
  echo "GitHub App installation ID must be a positive decimal number" >&2
  exit 1
fi

temporary_directory="$(mktemp -d)"
readonly temporary_directory
cleanup() {
  rm -rf "$temporary_directory"
}
trap cleanup EXIT
chmod 0700 "$temporary_directory"

decode_base64() {
  if base64 --decode </dev/null >/dev/null 2>&1; then
    base64 --decode
  else
    base64 -D
  fi
}

readonly app_id_file="$temporary_directory/app-id"
readonly private_key_file="$temporary_directory/private-key.pem"
readonly openai_key_file="$temporary_directory/api-key"
printf '%s' "$github_app_id" > "$app_id_file"
if ! printf '%s' "$github_app_private_key_b64" | decode_base64 > "$private_key_file"; then
  echo "GitHub App private key is not valid base64" >&2
  exit 1
fi
if ! grep -q -- '-----BEGIN.*PRIVATE KEY-----' "$private_key_file"; then
  echo "GitHub App private key does not contain a PEM private key" >&2
  exit 1
fi
printf '%s' "$openai_api_key" > "$openai_key_file"

context="$(dev_context)"
readonly context

kubectl --context "$context" apply -f - <<EOF
apiVersion: v1
kind: Namespace
metadata:
  name: $namespace
  labels:
    app.kubernetes.io/part-of: ayni-orchestrator
    orchestrator.ayni.dev/environment: local
EOF

apply_secret() {
  local name="$1"
  shift
  kubectl --context "$context" --namespace "$namespace" create secret generic "$name" \
    "$@" \
    --dry-run=client --output=yaml \
    | kubectl --context "$context" --namespace "$namespace" apply -f - >/dev/null
}

apply_secret github-app-credentials \
  --from-file=app-id="$app_id_file" \
  --from-file=private-key.pem="$private_key_file"
apply_secret openai-api --from-file=api-key="$openai_key_file"

if ! kubectl --context "$context" --namespace "$namespace" get secret development-publisher-auth >/dev/null 2>&1; then
  readonly publish_token_file="$temporary_directory/publish-token"
  openssl rand -hex 32 | tr -d '\n' > "$publish_token_file"
  apply_secret development-publisher-auth --from-file=token="$publish_token_file"
fi

echo "configured local development Secrets in namespace $namespace"
