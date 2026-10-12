# Ayni Orchestrator Helm chart

This reusable chart installs the Ayni Orchestrator control plane, operator web
application, namespace-scoped RBAC, and retained product claims. It does not
install Prometheus, Grafana, kube-state-metrics, local test fixtures, an Ingress,
or a public Service.

Supply deployable control-plane, worker, and web image references:

```sh
helm upgrade --install ayni-orchestrator deploy/charts/ayni-orchestrator \
  --namespace ayni-orchestrator \
  --create-namespace \
  --set-string images.controlPlane=REGISTRY/CONTROL_PLANE:TAG \
  --set runtime.enabled=true \
  --set-string runtime.workerImage=REGISTRY/WORKER@sha256:WORKER_DIGEST \
  --set-string images.web=REGISTRY/WEB:TAG
```

The target namespace must contain operator-managed Secrets required by the
enabled workloads:

- Optional administrator GitHub App: set `connections.githubAppSecretName` to a
  Secret containing `app-id` and `private-key.pem`. Leave empty for UI onboarding.
- `openai-api`: optional `api-key`
- Signing certificates can be imported or generated through the Certificates UI.
  To reference administrator-managed keys, set `connections.adminCertificates`
  to safe metadata `{id, displayName, secretName}`; the referenced Secret must
  contain `signing-key` (64 lowercase hexadecimal characters) and `key-id`.
  The controller snapshots immutable owned execution revisions; source Secrets
  remain administrator-owned. Signing material goes only to environment builds.
  Legacy `runtime.profiles.<profile>.build.signing_secret` and managed-runtime
  signing references are registered as read-only certificates for compatibility.
  Leave these fields empty for UI onboarding.

The chart deliberately does not create or accept credential values. Set
`images.pullPolicy` for the target registry and size
`persistence.controlPlaneState` before first installation. The controller
mounts only the clean database claim; pilot checkpoint, prepared-environment,
and managed-check-evidence claims are not created. Claims carry
`helm.sh/resource-policy: keep` and therefore survive release uninstall.

`customClients.enabled=true` adds the reviewed private TLS listener and requires
the Secret named by `customClients.secretName`; see
[`docs/custom-client-access.md`](../../../docs/custom-client-access.md). Each
namespace supports one installation because application resource names are
stable rather than release-prefixed.

Runtime limits are `codingTurns` (default 24) and `qualityRepairLimit` (default 3,
range 0–10). Admission freezes them per task. The worker owns checking, repairs
and draft publication; control-plane collection precedes cleanup.

Connections work with `runtime.enabled=false`. The control plane receives a
projected API token and namespaced Secret/Job permissions independently of runtime
execution. Set `connections.testImage` to the digest-pinned control-plane image
containing `registry-connection-test` to enable GHCR probes. The chart never accepts
credential values. UI-managed Secrets survive redeploys and remain separate from
admin synchronization. Schema-six/seven upgrades use the documented transactional migration;
never remove retained claims without explicit approval.

Register administrator GHCR connections explicitly with `connections.adminRegistries`:
`id`, `displayName`, `imageNamespace`, `username`, `pullSecretName` and
`pushSecretName`. These are safe metadata and references, never token values.
Use separate pull and push Secrets; labels cannot substitute for explicit registration.


## Automatic repository image preparation

`runtime.managedRuntime.build` configures engine infrastructure and service accounts.
Builder identity is resolved from the committed lock through the official compatibility
catalog. Selected certificates supply immutable signing references; selected registry
connections supply separate push and pull references. `runtime.engineMode` controls
the explicit engine boundary. See [repository builders](../../../docs/repository-builders.md).

Every successful environment build publishes and is independently verified before
Ready. Application and worker images remain deployment components. Upgrade using
cleaned values and `helm upgrade --install --reset-values`; removed global builder
fields are rejected by the schema. Preserve database and retained task-output claims.

Verification traces are optional. Set `telemetry.tracesEndpoint` to an
administrator-controlled OTLP/HTTP protobuf endpoint including `/v1/traces` and
restart the control plane. Empty disables export. The reusable chart does not
install a Collector or trace store; the Docker Desktop umbrella chart supplies
Collector, Jaeger and a Grafana datasource. Existing JSON logs and Prometheus
metrics remain independent. See [observability](../../../docs/observability-and-events.md).


Accepted-event publication is disabled by default. The trusted local profile may
set `outbox.natsUrl: nats://jetstream:4222` with `outbox.localDevelopment: true`.
The chart labels only the control-plane Pod as a broker client and supplies the
publisher configuration; it does not deploy a broker or give workers credentials.
Production authenticated transport remains separate. See
[publisher operations](../../../docs/local-development.md#persistent-local-jetstream).
