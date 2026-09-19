# Decision 0001: Docker Desktop Kubernetes and Kustomize

Status: accepted for the private MVP. This records the selected delivery approach;
deployment manifests and runtime validation will be implemented from M0 onward.

## Decision

Use Docker Desktop's Kubernetes support as the local deployment target. Use
Kustomize for the MVP's Kubernetes manifests, with a shared base and a
`docker-desktop` overlay. Deploy the first working control-plane service in M0
and exercise every subsequent deployable milestone locally before claiming it
complete.

Kustomize supports reusable bases and environment-specific overlays and can be
applied through `kubectl -k`. It fits one private deployment profile whose main
differences are image references, resources, configuration, and local access.
[Kubernetes Kustomize documentation](https://kubernetes.io/docs/tasks/manage-kubernetes-objects/kustomization/)

Helm provides chart templates, configurable values, and chart dependencies.
Those are useful when installation needs a packaged configuration interface.
For this pilot, maintain one set of manifests and revisit Helm when additional
installation environments create a concrete need. This supersedes the tentative
`deploy/chart/` directory in the repository-structure discussion.
[Helm chart documentation](https://helm.sh/docs/topics/charts/)

## Intended layout

Create these paths as their behavior is implemented; this decision does not
create empty scaffolding or deployable artifacts:

```text
deploy/
  base/                       # Workloads, services, configuration, permissions
  overlays/
    docker-desktop/           # Local namespace, images, resources, patches
  crds/                       # Generated definitions when introduced in M2
  images/                     # Control-plane and worker image definitions
  local/                      # Setup notes and local-only supporting resources
scripts/
  ...                         # Repeatable build, deploy, check, cleanup commands
```

Keep CRD generation deterministic and check generated definitions against their
Rust source. Establish namespaces and required CRDs before dependent resources.
Keep secrets and machine-specific values outside committed manifests.

## Local cluster and access

Docker Desktop documents kubeadm and kind provisioning. Prefer its kind
provisioner for a new compatible setup so the Kubernetes version can be selected;
retain an existing working Docker Desktop cluster unless a change is necessary.
Record the Docker Desktop, Kubernetes, provisioner, and client versions used.
Do not assume Docker Engine installation alone means this cluster is enabled.
[Docker Desktop Kubernetes documentation](https://docs.docker.com/desktop/use-desktop/kubernetes/)

Use the explicit `docker-desktop` context for every cluster operation and a
dedicated `ayni-orchestrator-dev` namespace for namespaced resources. The local
workflow must stop when that context is absent or the cluster cannot be verified;
it must never fall back to the current context. Avoid changing the user's global
context. Namespace scoping does not turn cluster-scoped CRDs into local objects.

Use internal Services and loopback port forwarding for initial health/API/UI
checks. Public ingress, public image publication, and cloud deployment are outside
this workflow. Enabling the selected local cluster is part of setup; resetting a
populated cluster or deleting retained data requires a separate explicit decision
because it can remove unrelated work.

## Build and deployment loop

For every deployable milestone:

1. Build relevant images from the checkout being reviewed and record source/image
   identity. Use a new content-specific reference for changed builds; record
   dirty-checkout provenance if applicable.
2. Make images available to the Kubernetes runtime through a verified local load
   path or private/local registry. A successful Docker build alone does not prove
   that a node can resolve the image. Verify the image actually used by the pod
   and its platform.
3. Render the overlay, inspect its resource changes, and validate it against the
   local API after required namespaces/CRDs are established. Record the renderer
   version so local and CI manifest checks are reproducible.
4. Apply through the explicit local context, wait for readiness, and run the
   milestone's behavior checks. Confirm health diagnostics and shutdown;
   restart/recovery checks grow with the implemented lifecycle.
5. Retain bounded logs, results, versions, and evidence references for the PR.
   Commit a concise evidence summary; keep generated artifacts and secrets out
   of source control.
6. Exercise scoped cleanup. Preserve retained evidence and other active work;
   destructive data removal must be explicit, not an update or normal cleanup
   side effect.

M0 provides the tested commands and scripts for this loop. Planning documents do
not count as a successful rollout. Private local deployments are part of the
user's standing development direction and should proceed once there is
deployable behavior and the target is verified.

## Managed-worker boundary

Docker Desktop Kubernetes is the orchestrator deployment target. Ayni's managed
Docker/Podman requirements remain a separate worker integration problem. A pod
cannot be assumed to contain an engine or access a node daemon.

M1 must select and exercise an isolated worker/engine arrangement that preserves
prepared dependency outputs, mounts, executor/image identity, bounded snapshots,
resource/capability policy, and final evidence collection. Keep engine privileges
away from the control plane and avoid sharing a host daemon with agent-controlled
work. Record privileges the arrangement actually requires and the limits of the
trusted local pilot.

An isolated Linux worker experiment can establish a baseline while investigating
the in-cluster arrangement. Report its topology honestly; do not claim native
pod execution or managed compatibility based on host-mode checks. If local
infrastructure blocks the selected path, retain the evidence and mark the
milestone incomplete rather than changing the acceptance contract.

## Consequences and revisit criteria

- The MVP has a single manifest source and local overlay; Helm packaging is deferred.
- Kustomize does not define application migration, rollback, or data-retention
  semantics. Document and test those as persistent state is introduced.
- Revisit Helm for actual installation requirements, such as independent
  deployments needing a stable values interface. Preserve one authoritative
  resource definition strategy when changing packaging.
- Local success establishes the tested Docker Desktop setup. Other clusters,
  architectures, multi-node behavior, hostile tenancy, and public distribution
  require separate evidence.
