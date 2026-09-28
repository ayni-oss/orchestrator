# Decision 0001: Docker Desktop Kubernetes and Kustomize

- Status: superseded by [Decision 0006](0006-local-helm-chart.md)
- Date: 2026-09-17
- Superseded: 2026-09-28

## Decision

Use Docker Desktop Kubernetes as the supported local deployment target and
Kustomize as the manifest composition tool. Maintain one shared base and one
`docker-desktop` overlay. Use the explicit `docker-desktop` context for every
cluster operation and deploy namespaced resources to
`ayni-orchestrator-dev` by default.

The local scripts must stop when that context is absent or unreachable. They
must not change the user's current context, reset the cluster, delete retained
state, or assume that a successful host Docker build makes an image available
to Kubernetes.

Use internal Services and loopback port-forwarding for API and browser access.
Public ingress, public image publication, and cloud deployment are outside this
local profile.

## Image and deployment identity

For every deployable change:

1. Build control-plane, web, and worker images from a clean committed checkout.
2. Tag and label them with the source revision.
3. Load them into Docker Desktop's Kubernetes node explicitly.
4. Render and inspect the Kustomize overlay.
5. Apply through the explicit local context and wait for readiness.
6. Verify the control-plane and web image identities actually used by the
   running Pod; the worker identity is frozen into generated Jobs.
7. Run the affected behavior checks and preserve only concise results in the
   linked pull request.

Generated logs, local credentials, and retained runtime artifacts do not belong
in Git.

## Worker boundary

A Kubernetes pod cannot be assumed to contain a container engine. Managed Ayni
execution therefore uses an attempt-local Docker-in-Docker sidecar. The engine
is privileged, but it is not shared with the control plane, the host Docker
socket, or repository-controlled processes. The worker remains non-root and has
no Kubernetes service-account token.

## Storage and cleanup

SQLite state, checkpoints, managed evidence, and prepared environments use
retained persistent volume claims. Normal cleanup removes only stateless service
resources. Namespace deletion, cluster reset, volume deletion, and broad Docker
pruning are separate destructive operator actions.

## Consequences

- This profile is intentionally local, single-cluster, and single-node.
- Kustomize is the authoritative manifest source; Helm is not maintained in
  parallel.
- Helm can be reconsidered when independently configured installations require a
  stable values interface.
- Multi-node storage, hostile tenancy, cloud deployment, and public distribution
  require separate designs and validation.
