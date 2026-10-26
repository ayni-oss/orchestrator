#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
reusable_chart="$repository_root/deploy/charts/ayni-orchestrator"
local_chart="$repository_root/dev/charts/ayni-orchestrator-local"

command -v helm >/dev/null 2>&1 || { echo "helm is required" >&2; exit 1; }
export HELM_NO_PLUGINS=1
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
export HELM_REPOSITORY_CONFIG="$scratch/repositories.yaml"
export HELM_REPOSITORY_CACHE="$scratch/cache"
export HELM_PLUGINS="$scratch/plugins"
mkdir -p "$scratch/cache" "$scratch/plugins"

control_image=local/control-plane:test
executor_image=local/conversation-supervisor:test
helm dependency build "$local_chart" >/dev/null
helm lint "$reusable_chart" \
  --set-string images.controlPlane="$control_image" \
  --set-string images.conversationSupervisor="$executor_image" >/dev/null
helm lint "$local_chart" \
  --set-string ayni-orchestrator.images.controlPlane="$control_image" \
  --set-string ayni-orchestrator.images.conversationSupervisor="$executor_image" >/dev/null

reusable_render="$scratch/reusable.yaml"
local_render="$scratch/local.yaml"
helm template reusable "$reusable_chart" --namespace test \
  --set-string images.controlPlane="$control_image" \
  --set-string images.conversationSupervisor="$executor_image" > "$reusable_render"
helm template local "$local_chart" --namespace test \
  --set-string ayni-orchestrator.images.controlPlane="$control_image" \
  --set-string ayni-orchestrator.images.conversationSupervisor="$executor_image" > "$local_render"

# The reusable chart consumes administrator-owned immutable Secrets and never
# renders a database, broker, or credential value.
grep -q 'name: workflow-postgres-migrator' "$reusable_render"
grep -q 'name: workflow-postgres-runtime' "$reusable_render"
grep -q 'ORCHESTRATOR_WORKFLOW_MIGRATION_DATABASE_URL_PATH' "$reusable_render"
grep -q 'ORCHESTRATOR_WORKFLOW_DATABASE_URL_PATH' "$reusable_render"
grep -q 'ORCHESTRATOR_WORKFLOW_DATABASE_ROLE: "ayni_runtime"' "$reusable_render"
grep -q 'name: workflow-jetstream-application' "$reusable_render"
grep -q 'name: workflow-jetstream-executor' "$reusable_render"
grep -q 'ORCHESTRATOR_WORKFLOW_BROKER_URL: "nats://jetstream:4222"' "$reusable_render"
grep -q 'ORCHESTRATOR_EXECUTOR_LEDGER_PATH' "$reusable_render"
grep -q 'helm.sh/resource-policy: keep' "$reusable_render"
! grep -q '^kind: Secret$' "$reusable_render"
! grep -q 'postgresql://' "$reusable_render"
! grep -q '^  nats.conf:' "$reusable_render"
! grep -qi 'sqlite' "$reusable_render"
! grep -q 'name: web' "$reusable_render"

migration_container="$(awk '/^      initContainers:/{capture=1} /^      containers:/{if(capture){exit}} capture{print}' "$reusable_render")"
control_container="$(awk '/^        - name: control-plane$/{capture=1} /^      volumes:/{if(capture){exit}} capture{print}' "$reusable_render")"
executor_container="$(awk '/^        - name: conversation-supervisor$/{capture=1} /^      volumes:/{if(capture){exit}} capture{print}' "$reusable_render")"
grep -q 'name: workflow-migrate' <<<"$migration_container"
grep -q 'workflow-postgres-migrator' <<<"$migration_container"
! grep -q 'workflow-postgres-runtime' <<<"$migration_container"
! grep -q 'jetstream' <<<"$migration_container"
grep -q 'name: control-plane' <<<"$control_container"
grep -q 'workflow-postgres-runtime' <<<"$control_container"
grep -q 'workflow-jetstream-application' <<<"$control_container"
! grep -q 'workflow-postgres-migrator' <<<"$control_container"
! grep -q 'workflow-jetstream-executor' <<<"$control_container"
grep -q 'name: conversation-supervisor' <<<"$executor_container"
grep -q 'workflow-jetstream-executor' <<<"$executor_container"
! grep -q 'postgres' <<<"$executor_container"
! grep -q 'workflow-jetstream-application' <<<"$executor_container"

# The trusted workflow listener remains container-local and absent from Service.
grep -q 'ORCHESTRATOR_GRPC_BIND_ADDRESS: "127.0.0.1:50051"' "$reusable_render"
service_block="$(awk '/^kind: Service$/{in_service=1} /^---$/{if(in_service){exit}} in_service{print}' "$reusable_render")"
! grep -q 'trusted-grpc' <<<"$service_block"

# The local chart pins stateful dependencies, renders the isolated three-stream
# profile, and injects four distinct broker roles only through Secret refs.
grep -q 'image: "postgres:17.6-bookworm@sha256:f3bd19c606e442c3d7bdfa8002e03fe260a1023351e0ea4598032022b68dd6e3"' "$local_render"
grep -q 'image: "nats:2.15.0-alpine@sha256:ac8f88a6494bffc2c2a5289a0ca61cb28a9145c11ba5677cf24265d07f46d8d4"' "$local_render"
grep -q 'image: "natsio/nats-box:0.20.0@sha256:d2f04223f40e016f77922a8115d34b0cd5fad2ec10b19b39316ae92d07f3d295"' "$local_render"
grep -q 'defaultMode: 0555' "$local_render"
grep -q "stat -c '%u:%g' /var/lib/postgresql/data" "$local_render"
grep -q 'chown -R 999:999 /var/lib/postgresql/data' "$local_render"
! grep -q 'ALTER DEFAULT PRIVILEGES' "$local_render"
grep -q 'max_payload: 327680' "$local_render"
for stream in ORCHESTRATOR_WORKFLOW_COMMAND_V1 ORCHESTRATOR_WORKFLOW_REPORT_V1 ORCHESTRATOR_WORKFLOW_DECISION_V1; do
  grep -q "$stream" "$local_render"
done
grep -q '"max_bytes": 268435456' "$local_render"
grep -q '"max_bytes": 201326592' "$local_render"
grep -q '"max_bytes": 67108864' "$local_render"
grep -q '"discard": "new"' "$local_render"
grep -q '"deny_delete": true' "$local_render"
grep -q '"deny_purge": true' "$local_render"
grep -q 'user: "workflow-application"' "$local_render"
grep -q 'user: "workflow-executor"' "$local_render"
grep -q 'user: "workflow-acceptance"' "$local_render"
grep -q 'user: "workflow-admin"' "$local_render"
grep -q 'secretKeyRef:' "$local_render"
grep -q 'command: \["/bin/sh", "/config/start.sh"\]' "$local_render"
grep -q 'quote_nats_string' "$local_render"
grep -q 'exec nats-server' "$local_render"
grep -q 'helm.sh/resource-policy: keep' "$local_render"
! grep -q 'postgresql://' "$local_render"
! grep -Eqi 'password: [a-f0-9]{32,}' "$local_render"
! grep -q 'name: web' "$local_render"

# Schemas and render-time guards reject unsafe role aliasing and unpinned inputs.
if helm template invalid "$reusable_chart" \
  --set-string postgres.runtimeSecretName= >/dev/null 2>&1; then
  echo "reusable chart accepted an empty runtime Secret name" >&2
  exit 1
fi
if helm template invalid "$reusable_chart" \
  --set-string postgres.runtimeSecretName=workflow-postgres-migrator >/dev/null 2>&1; then
  echo "reusable chart accepted one Secret for migration and runtime roles" >&2
  exit 1
fi
if helm template invalid "$reusable_chart" \
  --set-string jetstream.executorSecretName=workflow-jetstream-application >/dev/null 2>&1; then
  echo "reusable chart accepted one Secret for application and executor roles" >&2
  exit 1
fi
if helm template invalid "$reusable_chart" \
  --set controlPlane.httpPort=50051 --set controlPlane.trustedGrpcPort=50051 >/dev/null 2>&1; then
  echo "reusable chart accepted overlapping HTTP and trusted gRPC ports" >&2
  exit 1
fi
if helm template invalid "$local_chart" \
  --set-string postgres.secrets.runtime=workflow-postgres-migrator >/dev/null 2>&1; then
  echo "local chart accepted one Secret for migration and runtime roles" >&2
  exit 1
fi
if helm template invalid "$local_chart" \
  --set-string postgres.image=postgres:latest >/dev/null 2>&1; then
  echo "local chart accepted an unpinned PostgreSQL image" >&2
  exit 1
fi
if helm template invalid "$local_chart" \
  --set-string jetstream.serverImage=nats:latest >/dev/null 2>&1; then
  echo "local chart accepted an unpinned NATS image" >&2
  exit 1
fi
if helm template invalid "$local_chart" \
  --set-string jetstream.secrets.executor=workflow-jetstream-application >/dev/null 2>&1; then
  echo "local chart accepted aliased JetStream role Secrets" >&2
  exit 1
fi
if helm template invalid "$local_chart" \
  --set-string jetstream.secrets.admin=workflow-postgres-admin >/dev/null 2>&1; then
  echo "local chart accepted aliased PostgreSQL and JetStream role Secrets" >&2
  exit 1
fi
if helm template invalid "$local_chart" \
  --set-string ayni-orchestrator.postgres.runtimeSecretName=other-runtime >/dev/null 2>&1; then
  echo "local chart accepted mismatched reusable-workload and PostgreSQL Secrets" >&2
  exit 1
fi

bash -n \
  "$repository_root/dev/ctl" \
  "$repository_root/dev/lib/common.sh" \
  "$repository_root/dev/lib/images.sh" \
  "$repository_root/dev/scripts/configure-development-secrets.sh" \
  "$repository_root/dev/scripts/deploy.sh" \
  "$repository_root/dev/scripts/check.sh"

# Real calls require deployment opt-in; credentials are still mounted only by
# kubelet into the assigned session, never rendered as values or worker env.
grep -q 'name: session-authority' "$reusable_render"
grep -q 'name: ORCHESTRATOR_SESSION_AUTHORITY_URL' <<<"$executor_container"
grep -q 'name: ORCHESTRATOR_SESSION_AUTHORITY_TOKEN_PATH' <<<"$control_container"
grep -q 'name: ORCHESTRATOR_SESSION_AUTHORITY_TOKEN_PATH' <<<"$executor_container"
grep -q 'secretName: "workflow-session-authority"' "$reusable_render"
! grep -q 'name: OPENAI_BASE_URL' "$reusable_render"
! grep -q 'key: api-key' "$reusable_render"
! grep -q 'port: 443' "$reusable_render"
grep -A1 'name: ORCHESTRATOR_OPENAI_ENABLED' "$reusable_render" | grep -q 'value: "false"'
provider_render="$scratch/provider.yaml"
helm template provider "$reusable_chart" --namespace test \
  --set providers.openai.enabled=true > "$provider_render"
grep -q 'secretName: "workflow-session-authority"' "$provider_render"
grep -q 'ORCHESTRATOR_SESSION_AUTHORITY_TOKEN_PATH' "$provider_render"
grep -q 'value: "https://api.openai.com/v1"' "$provider_render"
grep -q 'port: 443' "$provider_render"
! grep -q 'key: api-key' "$provider_render"
! grep -q '^kind: Secret$' "$provider_render"
fixture_render="$scratch/fixture.yaml"
helm template fixture "$reusable_chart" --namespace test \
  --set providers.openai.enabled=true --set providers.openai.fixtureEgress=true \
  --set-string providers.openai.baseUrl=http://provider-fixture.test.svc:8080/v1 > "$fixture_render"
grep -q 'app: provider-fixture' "$fixture_render"
! grep -q 'cidr: 0.0.0.0/0' "$fixture_render"
if helm template invalid "$reusable_chart" \
  --set providers.openai.enabled=true --set-string providers.openai.baseUrl=http://arbitrary.example/v1 >/dev/null 2>&1; then
  echo "provider chart accepted cleartext outside its scoped fixture profile" >&2
  exit 1
fi
if helm template invalid "$reusable_chart" \
  --set providers.openai.enabled=true --set providers.openai.fixtureEgress=true \
  --set-string providers.openai.baseUrl=http://arbitrary.example/v1 >/dev/null 2>&1; then
  echo "provider fixture profile accepted an unrelated endpoint" >&2
  exit 1
fi
if helm template invalid "$reusable_chart" \
  --set-string providers.sessionAuthoritySecretName=workflow-jetstream-executor >/dev/null 2>&1; then
  echo "provider chart accepted aliased authority and broker Secrets" >&2
  exit 1
fi

# Publication has a separate, scoped capability and never inherits worker credentials.
publication_options=(
  --set publication.enabled=true
  --set-string publication.workspaceId=11111111-1111-4111-8111-111111111111
  --set-string publication.repository=https://github.com/fixture/repo.git
  --set-string publication.githubSecretName=publication-fixture-github
)
publication_render="$scratch/publication.yaml"
helm template publication "$reusable_chart" --namespace test "${publication_options[@]}" > "$publication_render"
publisher_container="$(awk '/^        - name: publisher$/{capture=1} /^      volumes:/{if(capture){exit}} capture{print}' "$publication_render")"
grep -q 'ayni-task-publisher' <<<"$publisher_container"
grep -q 'mountPath: /var/run/publication-authority' <<<"$publisher_container"
grep -q 'mountPath: /var/run/publication-github' <<<"$publisher_container"
! grep -Eq 'postgres|jetstream|session-authority|OPENAI' <<<"$publisher_container"
grep -q 'strategy: { type: Recreate }' "$publication_render"
grep -q 'name: task-publisher-isolation' "$publication_render"
for aliased_secret in workflow-publication-authority workflow-session-authority workflow-postgres-runtime workflow-jetstream-executor; do
  if helm template invalid "$reusable_chart" "${publication_options[@]}" \
    --set-string publication.githubSecretName="$aliased_secret" >/dev/null 2>&1; then
    echo "publication chart accepted an aliased capability Secret" >&2
    exit 1
  fi
done
if helm template invalid "$reusable_chart" --namespace test "${publication_options[@]}" \
  --set publication.fixtureEnabled=true \
  --set-string publication.fixtureApiUrl=http://unrelated.example:8080 \
  --set-string publication.fixtureGitUrl=git://publication-fixture.test.svc:9418/fixture/repo.git >/dev/null 2>&1; then
  echo "publication fixture accepted an unrelated endpoint" >&2
  exit 1
fi

echo "Helm authenticated workflow and provider isolation checks passed"

# Optional diagnostics stay internal and do not add credential or public access.
collector_render="$scratch/collector.yaml"
helm template traced "$reusable_chart" --namespace test \
  --set observability.collector.enabled=true \
  --set-string images.controlPlane="$control_image" \
  --set-string images.conversationSupervisor="$executor_image" > "$collector_render"
grep -q 'name: workflow-traces' "$collector_render"
grep -q 'http://workflow-traces.test.svc:4318/v1/traces' "$collector_render"
grep -q 'otel/opentelemetry-collector-contrib:0.147.0@sha256:e7c92c715f28ff142f3bcaccd4fc5603cf4c71276ef09954a38eb4038500a5a5' "$collector_render"
grep -q 'emptyDir: { sizeLimit: 32Mi }' "$collector_render"
grep -q 'name: reader' "$collector_render"
# Exporter configuration belongs to ConfigMap data; Kubernetes rejects metadata keys.
python3 - "$collector_render" <<'PYTHON'
import pathlib,sys
text=pathlib.Path(sys.argv[1]).read_text()
config=next(doc for doc in text.split("\n---") if "kind: ConfigMap\n" in doc and "name: control-plane-config\n" in doc)
metadata,data=config.split("\ndata:\n",1)
assert "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT" not in metadata
assert '  OTEL_EXPORTER_OTLP_TRACES_ENDPOINT: "http://workflow-traces.test.svc:4318/v1/traces"' in data
PYTHON
grep -q 'type: ClusterIP' "$collector_render"
! grep -q 'type: NodePort\|type: LoadBalancer' "$collector_render"
! grep -q 'OTEL_EXPORTER_OTLP_TRACES_ENDPOINT' "$reusable_render"
python3 "$repository_root/scripts/ci/test-execution-traces.py"

grep -q 'name: workflow-traces-isolation' "$collector_render"
grep -q 'app.kubernetes.io/name: ayni-workflow-traces' "$collector_render"
grep -q 'automountServiceAccountToken: false' "$collector_render"
grep -q 'max_megabytes: 8' "$collector_render"
grep -q 'max_backups: 2' "$collector_render"

# The publisher must be able to export its attempt-correlated reconciliation span.
# Test both real GitHub and fixture isolation profiles: each must allow only the
# internal collector selector on OTLP, and the default must retain its isolation.
for publication_fixture in false true; do
  traced_publication="$scratch/traced-publication-$publication_fixture.yaml"
  helm template traced "$reusable_chart" --namespace test "${publication_options[@]}" \
    --set observability.collector.enabled=true \
    --set publication.fixtureEnabled="$publication_fixture" \
    --set-string publication.fixtureApiUrl=http://publication-fixture.test.svc:8080 \
    --set-string publication.fixtureGitUrl=git://publication-fixture.test.svc:9418/fixture/repo.git > "$traced_publication"
  python3 - "$traced_publication" "$publication_render" <<'PYTHON'
import pathlib, sys

def document(path, name, kind):
    return next(doc for doc in pathlib.Path(path).read_text().split("\n---")
                if f"kind: {kind}\n" in doc and f"  name: {name}\n" in doc)

policy = document(sys.argv[1], "task-publisher-isolation", "NetworkPolicy")
expected = """    - to:
        - namespaceSelector:
            matchLabels:
              kubernetes.io/metadata.name: test
          podSelector:
            matchLabels:
              app.kubernetes.io/name: ayni-workflow-traces
      ports:
        - { protocol: TCP, port: 4318 }"""
assert expected in policy, "publisher OTLP endpoint has no scoped collector egress"
assert policy.count("port: 4318") == 1
assert "port: 4318" not in document(sys.argv[2], "task-publisher-isolation", "NetworkPolicy")
deployment = document(sys.argv[1], "task-publisher", "Deployment")
assert 'value: "http://workflow-traces.test.svc:4318/v1/traces"' in deployment
PYTHON
done
