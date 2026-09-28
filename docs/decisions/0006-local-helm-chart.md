# Decision 0006: Local Helm chart

- Status: accepted
- Date: 2026-09-28
- Supersedes: [Decision 0001](0001-local-kubernetes-deployment.md)

## Decision

Use Docker Desktop Kubernetes as the supported local deployment target and the
`deploy/charts/local` Helm chart as the single authoritative manifest source.
The chart installs the control plane, web application, retained claims, and the
disposable Prometheus, kube-state-metrics, and Grafana stack as one local
release. Use the explicit `docker-desktop` context for every cluster operation
and deploy namespaced resources to `ayni-orchestrator-dev` by default.

The local scripts must stop when that context is absent or unreachable. They
must not change the user's current context, reset the cluster, delete retained
state, or assume that a successful host Docker build makes an image available
to Kubernetes.

Use internal Services and loopback port-forwarding for application and
monitoring access. Public ingress, public image publication, and cloud
deployment are outside this local profile. Protected custom-client gRPC access
remains an opt-in chart value and does not weaken the default pod-loopback
listener.

## Image and deployment identity

For every deployable change:

1. Build control-plane, web, and worker images from a clean committed checkout.
2. Tag and label them with the source revision.
3. Load them into Docker Desktop's Kubernetes node explicitly.
4. Lint and render the `local` chart with the exact image values.
5. Upgrade the release through the explicit local context and wait for readiness.
6. Verify the control-plane and web image identities actually used by the
   running Pod; the worker identity is frozen into generated Jobs.
7. Run the application and observability checks and preserve only concise
   results in the linked pull request.

Generated manifests, logs, local credentials, and retained runtime artifacts do
not belong in Git.

## Migration and storage

The fixed `ayni-orchestrator` release adopts resources from the superseded
manifest workflow on its first upgrade. It preserves the existing resource
names and uses guarded ownership takeover plus server-side field-conflict
resolution rather than deleting and recreating resources. Operators must record
claim identities and task state
before the migration and verify both afterwards.

SQLite state, checkpoints, managed evidence, and prepared environments use
persistent volume claims annotated with `helm.sh/resource-policy: keep`. Normal
cleanup uninstalls the local release while preserving those claims, the
namespace, credentials, and task Jobs. Namespace deletion, cluster reset,
volume deletion, and broad Docker pruning are separate destructive operator
actions. Claim sizes are install-time minimums: expansion depends on the active
storage class, and reducing a claim size is not supported.

## Worker boundary

A Kubernetes pod cannot be assumed to contain a container engine. Managed Ayni
execution therefore uses an attempt-local Docker-in-Docker sidecar. The engine
is privileged, but it is not shared with the control plane, the host Docker
socket, or repository-controlled processes. The worker remains non-root and has
no Kubernetes service-account token.

## Consequences

- This chart is intentionally local, single-cluster, and single-node.
- Helm values are the supported configuration interface; a second manifest
  composition system is not maintained in parallel.
- Prometheus, kube-state-metrics, Grafana, and the provisioned dashboard are
  installed by default, but remain disposable operational views.
- Multi-node storage, hostile tenancy, cloud deployment, and public distribution
  require separate designs and validation.
