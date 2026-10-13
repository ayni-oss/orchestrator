#!/usr/bin/env bash
set -euo pipefail
readonly repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/bin" "$scratch/captured"
export BOOTSTRAP_TEST_LOG="$scratch/calls" BOOTSTRAP_TEST_CAPTURE="$scratch/captured"
: > "$BOOTSTRAP_TEST_LOG"
cat > "$scratch/bin/kubectl" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$BOOTSTRAP_TEST_LOG"
if [[ "$*" == 'config current-context' ]]; then echo docker-desktop; exit; fi
if [[ "$*" == *' get secret '* ]]; then
  if [[ "$*" == *'management-source}'* ]]; then
    case "$*" in
      *'github-app-credentials'*) printf '%s' "${MOCK_GITHUB_OWNER:-}" ;;
      *'runtime-push'*) printf '%s' "${MOCK_PUSH_OWNER:-}" ;;
      *'runtime-pull'*) printf '%s' "${MOCK_PULL_OWNER:-}" ;;
      *'openai-api'*) printf '%s' "${MOCK_OPENAI_OWNER:-}" ;;
    esac
  elif [[ "$*" == *'openai-api'* && "$*" == *'-o name'* ]]; then
    if [[ -n "${MOCK_OPENAI_OWNER:-}" ]]; then printf '%s' 'secret/openai-api'; fi
  elif [[ "$*" == *'image-namespace}'* ]]; then printf '%s' "${MOCK_IMAGE_NAMESPACE:-}";
  elif [[ "$*" == *'registry-username}'* ]]; then printf '%s' "${MOCK_REGISTRY_USERNAME:-}";
  fi
  exit
fi
if [[ "$*" == *' create secret generic '* ]]; then
  secret=''
  previous=''
  for argument in "$@"; do
    if [[ "$previous" == generic ]]; then secret="$argument"; fi
    if [[ "$argument" == --from-file=* ]]; then
      source="${argument#--from-file=}"; source="${source#*=}"
      if [[ "$argument" == --from-file=.dockerconfigjson=* ]]; then cp "$source" "$BOOTSTRAP_TEST_CAPTURE/$secret.json"; fi
    fi
    previous="$argument"
  done
  echo 'apiVersion: v1'
fi
if [[ "$*" == *' apply -f -'* ]]; then cat >/dev/null; fi
MOCK
cat > "$scratch/bin/helm" <<'MOCK'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$BOOTSTRAP_TEST_LOG"
MOCK
chmod +x "$scratch/bin/kubectl" "$scratch/bin/helm"
export PATH="$scratch/bin:$PATH"
run_configure() { bash "$repository_root/dev/scripts/configure-development-secrets.sh" --env-file "$1" >"$scratch/output" 2>"$scratch/errors"; }
run_configure "$scratch/missing.env"
[[ ! -s "$BOOTSTRAP_TEST_LOG" ]]
cat > "$scratch/incomplete.env" <<'DATA'
GITHUB_APP_ID=123
GHCR_PUSH_TOKEN=incomplete token with spaces
GHCR_PULL_USERNAME=incomplete username
AYNI_ENV_CERTIFICATE_KEY_ID=test-key
DATA
chmod 600 "$scratch/incomplete.env"
run_configure "$scratch/incomplete.env"
[[ ! -s "$BOOTSTRAP_TEST_LOG" ]]
cat > "$scratch/pull.env" <<'DATA'
GHCR_PULL_USERNAME=ExampleUser
GHCR_PULL_TOKEN=pull-test
DATA
chmod 600 "$scratch/pull.env"
run_configure "$scratch/pull.env"
[[ -f "$scratch/captured/runtime-pull.json" && ! -f "$scratch/captured/runtime-push.json" ]]
private_key_b64="$(printf '%s\n' '-----BEGIN PRIVATE KEY-----' 'fixture' '-----END PRIVATE KEY-----' | base64 | tr -d '\n')"
cat > "$scratch/complete.env" <<DATA
GITHUB_APP_ID=123
GITHUB_APP_PRIVATE_KEY_B64=$private_key_b64
OPENAI_API_KEY=openai-test
GHCR_PULL_USERNAME=ExampleUser
GHCR_PULL_TOKEN=pull-test
GHCR_PUSH_TOKEN=push-test
GHCR_IMAGE_NAMESPACE=ghcr.io/example-org
DATA
chmod 600 "$scratch/complete.env"
run_configure "$scratch/complete.env"
python3 - "$scratch/captured" <<'PY'
import base64,json,pathlib,sys
root=pathlib.Path(sys.argv[1])
for name,token in [('runtime-pull','pull-test'),('runtime-push','push-test')]:
    config=json.loads((root/f'{name}.json').read_text())
    assert base64.b64decode(config['auths']['ghcr.io']['auth']).decode()==f'ExampleUser:{token}'
PY
! grep -Eq 'pull-test|push-test|openai-test' "$BOOTSTRAP_TEST_LOG" "$scratch/output" "$scratch/errors"
sed '/^GHCR_IMAGE_NAMESPACE=/d' "$scratch/complete.env" > "$scratch/personal.env"
chmod 600 "$scratch/personal.env"
run_configure "$scratch/personal.env"
grep -Eq 'ayni.dev/image-namespace=ghcr.io/exampleuser' "$BOOTSTRAP_TEST_LOG"
export MOCK_GITHUB_OWNER=admin MOCK_PUSH_OWNER=admin MOCK_PULL_OWNER=admin MOCK_IMAGE_NAMESPACE=ghcr.io/example-org MOCK_REGISTRY_USERNAME=ExampleUser
source "$repository_root/dev/lib/connections.sh"
export MOCK_OPENAI_OWNER=admin
dev_bootstrap_helm_values docker-desktop test
[[ "$DEV_ADMIN_OPENAI_SECRET" == openai-api ]]
[[ "$DEV_ADMIN_GITHUB_SECRET" == github-app-credentials ]]
python3 - "$DEV_ADMIN_REGISTRIES" <<'PY'
import json,sys
registry,=json.loads(sys.argv[1])
assert registry['imageNamespace']=='ghcr.io/example-org'
assert registry['pushSecretName']=='runtime-push'
assert registry['pullSecretName']=='runtime-pull'
PY
helm upgrade "${DEV_BOOTSTRAP_HELM_VALUES[@]}"
[[ "$DEV_ADMIN_REGISTRIES" != '[]' ]]
export MOCK_PUSH_OWNER=ui MOCK_GITHUB_OWNER=ui MOCK_OPENAI_OWNER=ui
if run_configure "$scratch/complete.env"; then echo 'UI-managed credentials must not be overwritten' >&2; exit 1; fi
dev_bootstrap_connection_values docker-desktop test
[[ -z "$DEV_ADMIN_GITHUB_SECRET" && "$DEV_ADMIN_REGISTRIES" == '[]' && -z "$DEV_ADMIN_OPENAI_SECRET" ]]
export MOCK_PUSH_OWNER=admin MOCK_PULL_OWNER=''
dev_bootstrap_connection_values docker-desktop test
[[ "$DEV_ADMIN_REGISTRIES" == '[]' ]]
sed 's/GHCR_PUSH_TOKEN=push-test/GHCR_PUSH_TOKEN=pull-test/' "$scratch/complete.env" > "$scratch/same.env"
chmod 600 "$scratch/same.env"
if run_configure "$scratch/same.env"; then echo 'Distinct pull and push credentials are required' >&2; exit 1; fi
! grep -Eq 'pull-test|push-test|openai-test' "$BOOTSTRAP_TEST_LOG" "$scratch/output" "$scratch/errors"
printf '%s\n' 'OPENAI_API_KEY=openai-test' > "$scratch/openai.env"
chmod 600 "$scratch/openai.env"
if run_configure "$scratch/openai.env"; then echo 'UI-managed OpenAI credentials must not be overwritten' >&2; exit 1; fi
unset MOCK_OPENAI_OWNER MOCK_GITHUB_OWNER MOCK_PULL_OWNER MOCK_PUSH_OWNER
dev_bootstrap_helm_values docker-desktop test
[[ -z "$DEV_ADMIN_OPENAI_SECRET" && -z "$DEV_ADMIN_GITHUB_SECRET" && "$DEV_ADMIN_REGISTRIES" == '[]' ]]
helm upgrade "${DEV_BOOTSTRAP_HELM_VALUES[@]}"
grep -Eq 'ayni-orchestrator.connections.openaiSecretName= --set-json ayni-orchestrator.connections.adminRegistries=\[\]' "$BOOTSTRAP_TEST_LOG"
echo 'development credential bootstrap checks passed' 
