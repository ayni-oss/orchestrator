#!/usr/bin/env bash
# Registration reads only safe metadata from explicitly bootstrapped Secrets.
dev_bootstrap_connection_values() {
  local context="$1" namespace="$2" github_secret push_owner pull_owner image_namespace username openai_name openai_owner
  DEV_ADMIN_GITHUB_SECRET="${AYNI_ORCHESTRATOR_GITHUB_APP_SECRET_NAME:-}"
  DEV_ADMIN_REGISTRIES='[]'
  DEV_ADMIN_OPENAI_SECRET=''
  openai_name="$(kubectl --context "$context" --namespace "$namespace" get secret openai-api --ignore-not-found -o name)"
  if [[ -n "$openai_name" ]]; then
    openai_owner="$(kubectl --context "$context" --namespace "$namespace" get secret openai-api -o 'jsonpath={.metadata.labels.ayni\.dev/management-source}')"
    if [[ "$openai_owner" != ui ]]; then DEV_ADMIN_OPENAI_SECRET=openai-api; fi
  fi
  if [[ -z "$DEV_ADMIN_GITHUB_SECRET" ]]; then
    github_secret="$(kubectl --context "$context" --namespace "$namespace" get secret github-app-credentials --ignore-not-found -o 'jsonpath={.metadata.labels.ayni\.dev/management-source}')"
    if [[ "$github_secret" == admin ]]; then
      DEV_ADMIN_GITHUB_SECRET=github-app-credentials
    fi
  fi
  push_owner="$(kubectl --context "$context" --namespace "$namespace" get secret runtime-push --ignore-not-found -o 'jsonpath={.metadata.labels.ayni\.dev/management-source}')"
  pull_owner="$(kubectl --context "$context" --namespace "$namespace" get secret runtime-pull --ignore-not-found -o 'jsonpath={.metadata.labels.ayni\.dev/management-source}')"
  if [[ "$push_owner" != admin || "$pull_owner" != admin ]]; then
    return
  fi
  image_namespace="$(kubectl --context "$context" --namespace "$namespace" get secret runtime-push -o 'jsonpath={.metadata.annotations.ayni\.dev/image-namespace}')"
  username="$(kubectl --context "$context" --namespace "$namespace" get secret runtime-push -o 'jsonpath={.metadata.annotations.ayni\.dev/registry-username}')"
  if [[ ! "$image_namespace" =~ ^ghcr\.io/[a-z0-9]+([._-][a-z0-9]+)*$ || ${#image_namespace} -gt 100 || ! "$username" =~ ^[A-Za-z0-9-]{1,39}$ ]]; then
    echo "Skipping GHCR administrator registration without valid bootstrap metadata." >&2
    return
  fi
  DEV_ADMIN_REGISTRIES="$(python3 -c 'import json,sys; print(json.dumps([{"id":"registry-admin-local-ghcr","displayName":"Local GHCR","imageNamespace":sys.argv[1],"username":sys.argv[2],"pushSecretName":"runtime-push","pullSecretName":"runtime-pull"}],separators=(",",":")))' "$image_namespace" "$username")"
}

# Pass these safe reference-only values directly to Helm without an env file.
dev_bootstrap_helm_values() {
  dev_bootstrap_connection_values "$1" "$2"
  DEV_BOOTSTRAP_HELM_VALUES=(
    --set-string "ayni-orchestrator.connections.githubAppSecretName=$DEV_ADMIN_GITHUB_SECRET"
    --set-string "ayni-orchestrator.connections.openaiSecretName=$DEV_ADMIN_OPENAI_SECRET"
    --set-json "ayni-orchestrator.connections.adminRegistries=$DEV_ADMIN_REGISTRIES"
  )
}
