# Decision 0008: managed environment sourcing

- Status: accepted
- Date: 2026-10-02
- Supersedes: the Ayni Managed GHCR publication and GitHub Actions build model in Decision 0007

## Decision

Ayni Managed profiles obtain their runtime image by one of two explicit,
versioned sources:

1. **Registry source.** The operator supplies an OCI repository and discovery
   reference. The controller resolves and verifies the immutable digest and
   Ayni runtime metadata before admitting it.
2. **Orchestrator build source.** The controller creates one bounded,
   dedicated Kubernetes build Job for the profile revision. The Job builds from
   the committed Ayni lock and publishes the result to the profile's configured
   private OCI registry. The controller observes and verifies the published
   digest; it never executes image-building tooling itself.

Both sources produce the same admitted `repository@sha256:...` runtime
artifact. Coding and assessment Jobs only pull that frozen digest. They never
build an environment, receive build credentials, or inherit a container-engine
socket.

## Credentials and authority

The controller has no personal access token, package-write credential, Docker
socket, or registry secret material. Registry pull credentials stay on the
Kubernetes ServiceAccount or are supplied as short-lived, task-scoped pull
Secrets. Build push credentials, when the build source is selected, are mounted
only into the dedicated build Job from a Kubernetes Secret scoped to that Job's
ServiceAccount. They are not available to the controller, coding Jobs,
assessment Jobs, GitHub Actions, task state, evidence, metrics, or logs.

The build Job may use a cluster-approved builder implementation, but it must
not require privileged containers or a host Docker socket. Its output must be
addressable by an OCI registry because a node-local image is not a portable
runtime artifact. Docker Desktop image loading is permitted only as an explicit
single-node development distribution adapter; it is never evidence of artifact
identity or a substitute for registry publication on a multi-node topology.

## Admission and lifecycle

For either source, the controller reads the exact default-branch lock, derives
the full lock fingerprint, resolves the configured discovery reference, and
verifies registry ownership/policy, platform, immutable digest, Ayni labels,
runtime compatibility, and bounded build provenance. A mutable tag only
discovers an artifact; tasks retain the resolved digest.

A missing registry artifact leaves a registry-source profile non-ready. A
build-source profile creates or resumes its durable build action; a failed or
uncertain build is non-ready and is read back from the registry before any
retry. A source-only revision re-verifies the admitted artifact without a
build. A lock or source-policy change creates an updating revision: existing
attempts retain their frozen digest and new tasks wait for a newly admitted
artifact.

## Consequences

- GitHub Actions and GHCR are optional integrations, not required runtime
  infrastructure for Ayni Managed.
- Repository onboarding may install assessment automation, but it does not
  install a package-publication workflow or require `packages: write`.
- The old `lock-<fingerprint>` tag convention remains valid where an operator
  chooses it, but the configured registry/repository is authoritative and need
  not be GHCR.
- The build source needs a separate implementation and validation path for the
  approved in-cluster builder, registry Secret wiring, retries, and cleanup.
