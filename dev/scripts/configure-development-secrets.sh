#!/usr/bin/env bash
set -euo pipefail
umask 077

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
  echo "No development env file; configure connections through the UI or provide --env-file." >&2
  exit 0
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
ayni_environment_signing_key=""
ayni_environment_key_id=""
ghcr_pull_username=""
ghcr_pull_token=""
ghcr_push_token=""
ghcr_image_namespace=""
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
    AYNI_ENV_CERTIFICATE_SIGNING_KEY) ayni_environment_signing_key="$value" ;;
    AYNI_ENV_CERTIFICATE_KEY_ID) ayni_environment_key_id="$value" ;;
    GHCR_PULL_USERNAME) ghcr_pull_username="$value" ;;
    GHCR_PULL_TOKEN) ghcr_pull_token="$value" ;;
    GHCR_PUSH_TOKEN) ghcr_push_token="$value" ;;
    GHCR_IMAGE_NAMESPACE) ghcr_image_namespace="$value" ;;
    *)
      echo "unsupported environment-file key: $key" >&2
      exit 1
      ;;
  esac
done < "$env_file"

configure_github=false
if [[ -n "$github_app_id" || -n "$github_app_private_key_b64" ]]; then
  if [[ -n "$github_app_id" && -n "$github_app_private_key_b64" ]]; then
    configure_github=true
  else
    echo "Skipping incomplete GitHub credentials; finish setup in the UI or env file." >&2
  fi
fi
if [[ "$configure_github" == true && ! "$github_app_id" =~ ^[1-9][0-9]*$ ]]; then
  echo "GitHub App ID must be a positive decimal number" >&2
  exit 1
fi
if [[ "$configure_github" == true && -n "$github_app_installation_id" \
  && ! "$github_app_installation_id" =~ ^[1-9][0-9]*$ ]]; then
  echo "GitHub App installation ID must be a positive decimal number" >&2
  exit 1
fi

configure_ayni_environment_signing=false
if [[ -n "$ayni_environment_signing_key" && -n "$ayni_environment_key_id" ]]; then
  if [[ ! "$ayni_environment_signing_key" =~ ^[0-9a-f]{64}$ ]]; then
    echo "Ayni environment signing key must be a 32-byte lowercase hexadecimal seed" >&2
    exit 1
  fi
  if [[ ! "$ayni_environment_key_id" =~ ^[A-Za-z0-9._-]{1,128}$ ]]; then
    echo "Ayni environment certificate key ID must be 1 through 128 safe characters" >&2
    exit 1
  fi
  configure_ayni_environment_signing=true
elif [[ -n "$ayni_environment_signing_key" || -n "$ayni_environment_key_id" ]]; then
  echo "Skipping incomplete Ayni signing credentials; finish setup in the UI or env file." >&2
fi

configure_ghcr_pull=false
configure_ghcr_push=false
if [[ -n "$ghcr_pull_username" && -n "$ghcr_pull_token" ]]; then
  configure_ghcr_pull=true
elif [[ -n "$ghcr_pull_username" || -n "$ghcr_pull_token" ]]; then
  echo "Skipping incomplete GHCR pull credentials; finish setup in the UI or env file." >&2
fi
if [[ "$configure_ghcr_pull" == true ]]; then
  if [[ ! "$ghcr_pull_username" =~ ^[A-Za-z0-9-]{1,39}$ ]]; then
    echo "GHCR pull username must be a GitHub username" >&2
    exit 1
  fi
fi
tokens_to_validate=()
if [[ "$configure_ghcr_pull" == true ]]; then tokens_to_validate+=("$ghcr_pull_token"); fi
if [[ "$configure_ghcr_pull" == true && -n "$ghcr_push_token" ]]; then tokens_to_validate+=("$ghcr_push_token"); fi
for token in "${tokens_to_validate[@]}"; do
  if [[ ${#token} -gt 4096 || "$token" =~ [[:space:]] ]]; then
    echo "GHCR tokens must be at most 4096 characters with no whitespace" >&2
    exit 1
  fi
done
if [[ "$configure_ghcr_pull" == true && -n "$ghcr_push_token" ]]; then
  if [[ "$ghcr_push_token" == "$ghcr_pull_token" ]]; then
    echo "GHCR requires distinct push and pull credentials" >&2
    exit 1
  fi
  if [[ -z "$ghcr_image_namespace" ]]; then
    echo "GHCR_IMAGE_NAMESPACE is required for connection tests; set ghcr.io/<test-namespace> explicitly" >&2
    exit 1
  fi
  if [[ ${#ghcr_image_namespace} -gt 100 || ! "$ghcr_image_namespace" =~ ^ghcr\.io/[a-z0-9]+([._-][a-z0-9]+)*$ ]]; then
    echo "GHCR_IMAGE_NAMESPACE must be ghcr.io followed by a lowercase owner namespace" >&2
    exit 1
  fi
  configure_ghcr_push=true
elif [[ -n "$ghcr_push_token" ]]; then
  echo "Skipping incomplete GHCR connection; push requires separate pull credentials." >&2
fi
if [[ "$configure_github" == false && "$configure_ayni_environment_signing" == false \
  && "$configure_ghcr_pull" == false && -z "$openai_api_key" ]]; then
  echo "No complete development credentials; configure connections through the UI." >&2
  exit 0
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
readonly ayni_signing_key_file="$temporary_directory/ayni-signing-key"
readonly ayni_key_id_file="$temporary_directory/ayni-key-id"
readonly ghcr_dockerconfig_file="$temporary_directory/ghcr-dockerconfigjson"
readonly ghcr_push_dockerconfig_file="$temporary_directory/ghcr-push-dockerconfigjson"
if [[ "$configure_github" == true ]]; then
printf '%s' "$github_app_id" > "$app_id_file"
if ! printf '%s' "$github_app_private_key_b64" | decode_base64 > "$private_key_file"; then
  echo "GitHub App private key is not valid base64" >&2
  exit 1
fi
if ! grep -q -- '-----BEGIN.*PRIVATE KEY-----' "$private_key_file"; then
  echo "GitHub App private key does not contain a PEM private key" >&2
  exit 1
fi
fi
printf '%s' "$openai_api_key" > "$openai_key_file"
if [[ "$configure_ayni_environment_signing" == true ]]; then
  printf '%s' "$ayni_environment_signing_key" > "$ayni_signing_key_file"
  printf '%s' "$ayni_environment_key_id" > "$ayni_key_id_file"
fi
if [[ "$configure_ghcr_pull" == true ]]; then
  ghcr_auth="$(printf '%s:%s' "$ghcr_pull_username" "$ghcr_pull_token" | base64 | tr -d '\n')"
  printf '{"auths":{"ghcr.io":{"auth":"%s"}}}\n' "$ghcr_auth" > "$ghcr_dockerconfig_file"
  chmod 0600 "$ghcr_dockerconfig_file"
fi

if [[ "$configure_ghcr_push" == true ]]; then
  ghcr_push_auth="$(printf '%s:%s' "$ghcr_pull_username" "$ghcr_push_token" | base64 | tr -d '\n')"
  printf '{"auths":{"ghcr.io":{"auth":"%s"}}}\n' "$ghcr_push_auth" > "$ghcr_push_dockerconfig_file"
  chmod 0600 "$ghcr_push_dockerconfig_file"
fi

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
  local management
  management="$(kubectl --context "$context" --namespace "$namespace" get secret "$name" --ignore-not-found -o 'jsonpath={.metadata.labels.ayni\.dev/management-source}')"
  if [[ "$management" == "ui" ]]; then
    echo "Refusing to overwrite UI-managed Secret: $name" >&2
    return 1
  fi
  kubectl --context "$context" --namespace "$namespace" create secret generic "$name" \
    "$@" \
    --dry-run=client --output=yaml \
    | kubectl --context "$context" --namespace "$namespace" apply -f - >/dev/null
}

if [[ "$configure_github" == true ]]; then
apply_secret github-app-credentials \
  --from-file=app-id="$app_id_file" \
  --from-file=private-key.pem="$private_key_file"
kubectl --context "$context" --namespace "$namespace" label secret github-app-credentials ayni.dev/management-source=admin ayni.dev/connection-kind=github_app --overwrite >/dev/null
fi
if [[ -n "$openai_api_key" ]]; then
  apply_secret openai-api --from-file=api-key="$openai_key_file"
  kubectl --context "$context" --namespace "$namespace" label secret openai-api ayni.dev/management-source=admin ayni.dev/connection-kind=openai --overwrite >/dev/null
fi
if [[ "$configure_ayni_environment_signing" == true ]]; then
  apply_secret environment-producer \
    --from-file=signing-key="$ayni_signing_key_file" \
    --from-file=key-id="$ayni_key_id_file"
fi
if [[ "$configure_ghcr_pull" == true ]]; then
  apply_secret runtime-pull \
    --type=kubernetes.io/dockerconfigjson \
    --from-file=.dockerconfigjson="$ghcr_dockerconfig_file"
  kubectl --context "$context" --namespace "$namespace" label secret runtime-pull ayni.dev/management-source=admin ayni.dev/connection-kind=ghcr --overwrite >/dev/null
fi

if [[ "$configure_ghcr_push" == true ]]; then
  apply_secret runtime-push \
    --type=kubernetes.io/dockerconfigjson \
    --from-file=.dockerconfigjson="$ghcr_push_dockerconfig_file"
  kubectl --context "$context" --namespace "$namespace" label secret runtime-push ayni.dev/management-source=admin ayni.dev/connection-kind=ghcr --overwrite >/dev/null
  kubectl --context "$context" --namespace "$namespace" annotate secret runtime-push \
    "ayni.dev/image-namespace=$ghcr_image_namespace" "ayni.dev/registry-username=$ghcr_pull_username" --overwrite >/dev/null
fi

echo "configured local development Secrets in namespace $namespace"
