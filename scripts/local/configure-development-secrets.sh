#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
readonly namespace="ayni-orchestrator-dev"

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
    GITHUB_APP_ID_FILE|GITHUB_APP_INSTALLATION_ID|GITHUB_APP_PRIVATE_KEY_FILE|OPENAI_API_KEY_FILE|DEVELOPMENT_PUBLISH_TOKEN_FILE)
      settings["$key"]="$value"
      ;;
    *)
      echo "unsupported environment-file key: $key" >&2
      exit 1
      ;;
  esac
done < "$env_file"

for key in GITHUB_APP_ID_FILE GITHUB_APP_PRIVATE_KEY_FILE OPENAI_API_KEY_FILE DEVELOPMENT_PUBLISH_TOKEN_FILE; do
  if [[ -z "${settings[$key]:-}" ]]; then
    echo "missing required environment-file key: $key" >&2
    exit 1
  fi
done

readonly env_directory="$(cd "$(dirname "$env_file")" && pwd)"
resolve_path() {
  local value="$1"
  if [[ "$value" == /* ]]; then
    printf '%s\n' "$value"
  else
    printf '%s/%s\n' "$env_directory" "$value"
  fi
}

readonly app_id_file="$(resolve_path "${settings[GITHUB_APP_ID_FILE]}")"
readonly private_key_file="$(resolve_path "${settings[GITHUB_APP_PRIVATE_KEY_FILE]}")"
readonly openai_key_file="$(resolve_path "${settings[OPENAI_API_KEY_FILE]}")"
readonly publish_token_file="$(resolve_path "${settings[DEVELOPMENT_PUBLISH_TOKEN_FILE]}")"

for file in "$app_id_file" "$private_key_file" "$openai_key_file" "$publish_token_file"; do
  if [[ ! -f "$file" ]]; then
    echo "credential file does not exist: $file" >&2
    exit 1
  fi
done

if ! tr -d '\n' < "$app_id_file" | grep -Eq '^[1-9][0-9]*$'; then
  echo "GitHub App ID must be a positive decimal number" >&2
  exit 1
fi
if [[ "$(wc -c < "$publish_token_file" | tr -d ' ')" -lt 32 ]]; then
  echo "development publish token must contain at least 32 bytes" >&2
  exit 1
fi

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
apply_secret development-publisher-auth --from-file=token="$publish_token_file"
apply_secret openai-api --from-file=api-key="$openai_key_file"

echo "configured local development Secrets in namespace $namespace"
