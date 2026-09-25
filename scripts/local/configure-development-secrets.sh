#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
readonly namespace="${AYNI_ORCHESTRATOR_NAMESPACE:-ayni-orchestrator-dev}"

usage() {
  echo "usage: $0 [--env-file PATH]" >&2
}

env_file="$repository_root/.env"
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

declare -A settings=()
while IFS= read -r line || [[ -n "$line" ]]; do
  [[ -z "$line" || "$line" == \#* ]] && continue
  if [[ ! "$line" =~ ^([A-Z0-9_]+)=(.*)$ ]]; then
    echo "invalid environment-file entry" >&2
    exit 1
  fi
  key="${BASH_REMATCH[1]}"
  value="${BASH_REMATCH[2]}"
  case "$key" in
    GITHUB_APP_ID|GITHUB_APP_INSTALLATION_ID|GITHUB_APP_PRIVATE_KEY_B64|OPENAI_API_KEY)
      settings["$key"]="$value"
      ;;
    *)
      echo "unsupported environment-file key: $key" >&2
      exit 1
      ;;
  esac
done < "$env_file"

for key in GITHUB_APP_ID GITHUB_APP_PRIVATE_KEY_B64 OPENAI_API_KEY; do
  if [[ -z "${settings[$key]:-}" ]]; then
    echo "missing required environment-file key: $key" >&2
    exit 1
  fi
done
if [[ ! "${settings[GITHUB_APP_ID]}" =~ ^[1-9][0-9]*$ ]]; then
  echo "GitHub App ID must be a positive decimal number" >&2
  exit 1
fi
if [[ -n "${settings[GITHUB_APP_INSTALLATION_ID]:-}" \
  && ! "${settings[GITHUB_APP_INSTALLATION_ID]}" =~ ^[1-9][0-9]*$ ]]; then
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
printf '%s' "${settings[GITHUB_APP_ID]}" > "$app_id_file"
if ! printf '%s' "${settings[GITHUB_APP_PRIVATE_KEY_B64]}" | decode_base64 > "$private_key_file"; then
  echo "GitHub App private key is not valid base64" >&2
  exit 1
fi
if ! grep -q -- '-----BEGIN.*PRIVATE KEY-----' "$private_key_file"; then
  echo "GitHub App private key does not contain a PEM private key" >&2
  exit 1
fi
printf '%s' "${settings[OPENAI_API_KEY]}" > "$openai_key_file"

context="$($repository_root/scripts/local/require-docker-desktop-context.sh)"
readonly context

kubectl --context "$context" apply -f - <<EOF
apiVersion: v1
kind: Namespace
metadata:
  name: $namespace
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
