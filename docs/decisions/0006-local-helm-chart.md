# Decision 0006: Reusable application chart and local Helm composition

- Status: accepted
- Date: 2026-09-28
- Supersedes: [Decision 0001](0001-local-kubernetes-deployment.md)

## Historical scope

The Helm composition, explicit Docker Desktop context, and loopback deployment boundary remain accepted. Its pilot storage and Docker-in-Docker worker descriptions are historical; [Decision 0007](0007-clean-v1-runtime-profiles.md) defines the clean-v1 resource and runner model.

## Decision

Maintain a reusable application chart under `deploy/charts` and contain the
Docker Desktop development composition under `dev`:

- `deploy/charts/ayni-orchestrator` is the reusable, namespace-neutral application chart. It
  installs the control plane, operator web application, RBAC, optional protected
  custom-client listener, and retained product claims without a monitoring
  stack or Docker Desktop assumptions.
- `dev/charts/ayni-orchestrator-local` is the Docker Desktop development umbrella. It depends on the
  application chart and version-locked upstream Prometheus,
  kube-state-metrics, and Grafana charts, and adds local scrape configuration,
  least-privilege monitoring RBAC and the dashboard. Runtime proof fixtures
  are not packaged with the normal development release.

`dev/charts/ayni-orchestrator-local/Chart.lock` fixes the dependency chart versions selected by
the umbrella. Generated archives under `dev/charts/ayni-orchestrator-local/charts` are ignored;
local scripts build them from the lock before rendering or installing. Helm's
lock records repository coordinates and versions rather than archive checksums,
so upstream chart artifact immutability still depends on those repositories;
container images are pinned separately by digest. The upstream application
versions remain Prometheus 3.5.0, kube-state-metrics 2.16.0, and Grafana 12.1.1.

Use the explicit `docker-desktop` context for every local cluster operation and
deploy namespaced resources to `ayni-orchestrator-dev` by default. Local scripts
must stop when that context is absent or unreachable. They must not change the
user's current context, reset the cluster, delete retained state, or assume that
a successful host Docker build makes an image available to Kubernetes.

Use internal Services and loopback port-forwarding for application and
monitoring access. Public ingress, public image publication, and cloud
deployment are outside the local profile. Protected custom-client gRPC access
remains an opt-in application-chart value and does not weaken the default
pod-loopback listener.

## Image and deployment identity

For every deployable change:

1. Build control-plane, web, and worker images from the current checkout,
   including uncommitted local changes when present.
2. Label them with the source revision and whether the source checkout was dirty.
   Generated local tags include each image's content ID; dirty images are not
   reproducible from the commit revision alone.
3. Load them into Docker Desktop's Kubernetes node explicitly.
4. Build version-locked dependencies, then lint and render both charts with the
   exact application image values.
5. Upgrade the fixed local release through the explicit context and wait for
   readiness.
6. Verify the control-plane and web image identities actually used by the
   running Pod; the worker identity is frozen into generated Jobs.
7. Run the application and observability checks and preserve only concise
   results in the linked pull request.

Generated dependency archives, rendered manifests, logs, local credentials, and
retained runtime artifacts do not belong in Git.

## Migration and storage

The local environment is disposable and intentionally carries no compatibility
path for the superseded manifest workflow or older hand-written monitoring
resources. A collision with unmanaged or incompatible resources is resolved by
backing up anything needed, running the guarded `dev/ctl purge` command, and
creating a fresh Helm-managed environment. This keeps routine deployment small
and prevents historical takeover logic from becoming a permanent maintenance
surface.

SQLite state uses a persistent volume claim annotated with
`helm.sh/resource-policy: keep`. Normal cleanup uninstalls the local release while preserving that claim, the
namespace, credentials, and task Jobs. Namespace deletion, cluster reset,
volume deletion, and broad Docker pruning are separate destructive operator
actions. Claim sizes are install-time minimums: expansion depends on the active
storage class, and reducing a claim size is not supported.

## Worker boundary

A Kubernetes pod cannot be assumed to contain a container engine. Coding and
assessment containers pull a controller-admitted digest and start it through
the platform launcher; they do not build an Ayni environment, use
Docker-in-Docker, or mount a Docker socket. The worker remains non-root and has
no Kubernetes service-account token. See the [shared runtime runner](../runtime-runner.md)
and [Ayni 0.14 integration contract](../ayni-0.14-integration.md).

## Consequences

- The application chart can be installed and configured independently of the
  local observability stack.
- The local chart remains intentionally single-cluster and single-node, while
  composition and version selection are explicit in `Chart.yaml` and
  `Chart.lock`.
- Helm values are the supported configuration interface; a second manifest
  composition system is not maintained in parallel.
- Prometheus, kube-state-metrics, Grafana, and the provisioned dashboard are
  installed by the local umbrella by default but remain disposable operational
  views.
- Multi-node storage, hostile tenancy, cloud deployment, and public distribution
  require separate designs and validation.
