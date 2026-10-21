# Ayni Orchestrator Helm chart

This reusable chart deploys the durable control plane and retained conversation supervisor with isolated scripted or opt-in OpenAI sessions. PostgreSQL remains the workflow authority and authenticated JetStream provides durable transport. The chart does not install either dependency or create credentials.

## Required Secrets

Create five distinct immutable, administrator-owned Secrets before installing:

- `workflowPostgres.migratorSecretName`: a `url` key for the migration-owner role;
- `workflowPostgres.runtimeSecretName`: a `url` key for the least-privilege application role;
- `workflowJetstream.applicationSecretName`: a `password` key for the application broker identity;
- `workflowJetstream.executorSecretName`: `password`, `instance-id`, and `signing-seed-hex` keys owned only by the scripted executor; and
- `providers.sessionAuthoritySecretName`: a `token` key for authenticated session recovery and paid-call admission.

`workflowPostgres.runtimeRoleName` must name the role in the runtime URL. All Secret names must differ. Database URLs, broker passwords, and the executor signing seed never enter ConfigMaps or command arguments. The migration init container mounts only the migration URL, the serving container mounts the runtime URL plus its broker password, and the executor mounts its own broker/signing Secret and retained ledger.
The application and supervisor always mount the distinct session-authority bearer
Secret. Recovery authority remains available when new OpenAI calls are disabled.

The executor PVC retains exact commands and signed reports across restarts. The chart marks that claim with `helm.sh/resource-policy: keep`; deletion requires an explicit operator action after unresolved decisions are repaired or the disposable environment is intentionally reset.

## Deployment boundary

The supported deployment profile is an isolated trusted local environment. PostgreSQL and NATS transport are plaintext only within that boundary. Do not use shared or production credentials or traffic until verified TLS and the later operational acceptance work are complete.

## Listeners

- HTTP health and readiness are exposed through the `control-plane` Service.
- Canonical private-v1 gRPC binds to pod loopback only. Use an explicit pod port-forward or the in-pod diagnostic.
- The supervisor has no network listener or database credential. Its scoped Kubernetes service account provisions isolated sessions.
- No custom-client workflow listener is installed.

See `values.yaml` for the supported values surface.

## OpenAI opt-in

`providers.openai.enabled` defaults to `false`. The distinct administrator-owned
`providers.sessionAuthoritySecretName` Secret with a `token` key is required in
the application namespace even for the default scripted profile. This bearer
supports retained-result recovery, request admission and cancellation polling;
it is not a provider API key. Disabling new paid calls does not remove the
application listener, supervisor URL or bearer mounts needed to recover accepted work.

Provision each workspace's provider credential separately in the session namespace
as `provider-openai-<workspace UUID without hyphens>`, with an `api-key` key.
The kubelet mounts that Secret directly into that workspace's conversation workers.
Neither the application nor supervisor reads its data or forwards it through the
API or broker. The supervisor's preexisting dynamic bearer provisioning role can
read session-namespace Secrets; it remains denied application-namespace Secret
access. Only worker code reads its own mounted provider key.

`providers.openai.baseUrl` defaults to `https://api.openai.com/v1`. The production
profile allows DNS and public IPv4 TCP 443 egress, excluding private, loopback,
link-local and multicast ranges. NetworkPolicy enforcement depends on the cluster
network plugin. The operator endpoint must use HTTPS. No provider native tools,
provider-managed conversation state or automatic create-request retries are enabled.

For the deterministic local fixture, enable `providers.openai.fixtureEgress` and
use `http://provider-fixture.<application namespace>.svc:8080/v1`. This profile
permits only DNS and TCP 8080 to the labeled fixture Pod in the application
namespace; it grants no public provider egress. See the
[fixture instructions](../../../dev/fixtures/provider/README.md).
