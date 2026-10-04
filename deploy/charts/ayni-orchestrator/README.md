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

- `github-app-credentials`: `app-id` and `private-key.pem`
- `openai-api`: optional `api-key`
- For dedicated Kubernetes environment builds, the Secret referenced by
  `runtime.profiles.<profile>.build.signing_secret`: `signing-key` and `key-id`.
  The build Job references it directly; the chart does not mount it into the
  control-plane or web containers. The local development bootstrap can create
  `environment-producer` when the Ayni signing values are present in `dev/.env`.

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
