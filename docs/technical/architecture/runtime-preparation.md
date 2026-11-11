# Kubernetes runtime preparation

Runtime preparation produces a verified immutable image for one exact inspected
repository commit. It does not approve plans, start tasks or change task runtime
selection. The [repository preparation contract](repository-preparation.md) owns
those later handoffs.

## Signer setup and recovery

`ProvisionRuntimeSigner` accepts a public idempotency key, name, Key ID and
`generate` or `import` mode. Import accepts only a raw 32-byte Ed25519 seed encoded
as 64 lowercase hexadecimal characters, optionally followed by one LF. Generate
creates material on the server. The deployment-gated service creates an immutable
Kubernetes Secret in its signing namespace and registers its public metadata;
responses contain no private seed. Private material is not stored in PostgreSQL,
request receipts, browser storage or public operation history.

The browser retains only the public setup intent and clears its password/file
inputs before awaiting submission and again on completion. Recovery uses
`RecoverRuntimeSignerProvisioning` with the original idempotency key. Generated
setup adopts the winning Secret rather than creating a replacement. If an imported
Secret is absent, the retained public identity requires the same seed again;
re-entry cannot replace it with a different key. Pending and blocked operations
remain recoverable across reloads. Disabled deployment and permission failures
produce closed, actionable findings without echoing private request data.

The advanced reference editor remains available for administrator-provisioned
Secrets. Neither flow edits repository policy, grants build authority, nor changes
a selected runtime. Operators must review the public trust entry, commit it with a
regenerated lock and obtain a new exact-commit inspection.

Generated/imported signing Secrets and their namespace are retained when signer
setup is disabled or Helm is uninstalled, including deployments where runtime
preparation is off. Disabling a signer prevents new use; it does not delete its
private key. Deleting retained keys or the signing namespace is a separate manual
operator cleanup decision. There is no key-deletion API.

## Repository onboarding

`StartRepositoryRuntime` retains one reviewed, bounded default-branch preparation
intent and its original receipt in PostgreSQL. Source revision, registry/signer
revisions, platform and build/replacement allowances are captured. The control
plane claims a leased step, resolves the latest default branch once, persists the
exact SHA, runs canonical inspection and admits canonical runtime preparation.
The run has separate deterministic inspection/preparation keys. Restart, lost
responses and expired claims recover those commands; stale claims cannot advance
state. Admission checks the reviewed registry/signer revisions inside the same
transaction that freezes the preparation. Only public settings and references are
retained; this adds no new credential capability.

`GetRepositoryRuntime` reads current progress and last reviewed settings;
`GetRepositoryRuntimeReceipt` reads the immutable original acknowledgement.
Onboarding `completed` means the preparation has been admitted and linked, not
that an image was published or verified. Actual progress, cancellation, publication
reconciliation and readiness belong to the linked preparation. Blocking source,
setup, trust, configuration and deployment issues remain explicit. A failed step
cannot grant additional builds. New source or allowance requires a new reviewed
run; this feature does not install a push listener or recurring build schedule.

## Admission and evidence

`StartRuntimePreparation` freezes the completed inspection, repository/connection
revisions, exact commit/tree, policy/lock/input bytes, platform, registry destination,
signer reference and administrator-owned qualified profile. The requested seconds,
build count, build grant and invalid-image replacement grant are explicit.
Unqualified/absent profiles, stale revisions, unsupported locks and untrusted signer
keys produce retained actionable blockers rather than an alternative executor.

PostgreSQL owns operations, stage generations, receipts and accepted evidence.
A command retry recovers its exact receipt. Stage reports bind operation, stage,
generation, command digest and exact Kubernetes pod UID using derived capabilities.
A completed stage first enters `stopping`; no successor receives inputs or authority
until the supervisor confirms deletion of that exact pod. A stale pod/report cannot
replace a newer binding.

| Stage | Capability and output |
| --- | --- |
| Acquire source | Installation-scoped GitHub read; exact commit/tree and bounded complete checkout. Locked source is also retained separately. |
| Discover/pull | Destination-scoped registry read; reuse a retained immutable candidate or discover a platform candidate. Confirmed absence is different from access failure. |
| Build | Source read-only; rootless Podman with operation-local vfs storage and bounded public dependency access. No source, signer or push credential. |
| Freeze candidate | Independent verified OCI copy/unpack after builder deletion; freeze unsigned manifest and protected root. |
| Certify | Read-only frozen candidate and locked inputs; reviewed administrator signer Secret only. Ayni's public offline contract signs independently inspected protected bytes. |
| Finalize | Read-only unsigned export/certification layer/report; standard OCI tooling produces a retained intended manifest. No signing seed. |
| Publish | Final export plus destination push capability; intended digest is retained before publication. |
| Pull final | Independent registry readback of the intended immutable digest. |
| Verify content | Actual candidate filesystem plus trusted static verifier; no application credential or candidate CLI execution. |
| Verify compatibility | Credential-free candidate image; isolated preparation init container, then a separate managed quality container with frozen prepared source. |
| Cleanup | Delete owned pods, temporary credential objects and operation PVCs. Readiness waits for confirmed cleanup. |

Temporary image-pull credentials are delivered to the Kubernetes node, never
mounted into verification containers. Signing keys remain outside PostgreSQL and
worker credential APIs. The supervisor validates signer Secret immutability and
UID before launch and after certification; the certifier checks its public-key
agreement. Only then can the supervisor submit accepted certification evidence.

OCI descriptors, config/layers and transfers are digest checked and bounded.
Layer expansion is preflighted before unpacking, including decompressed bytes,
entries and unsupported sparse records. Generic OCI handling does not replace
Ayni's protected-content contract. Original source mode/content is checked before
builder execution and after managed preparation.

## Managed compatibility

The content verifier uses pinned Ayni public APIs and a separately trusted CLI hash
against the image's actual root. Only after this passes does the managed probe
invoke that image's expected CLI. Trusted static launchers are installed from the
pinned probe image, never copied from the repository.

Preparation and quality use separate container cgroups. Their trusted static parent
runs as UID/GID 0 inside the pod user namespace, with only SETUID/SETGID and
no-new-privileges. This is not host root. The parent requires those exact effective
capabilities before spawning the UID/GID 10002 child, which retains no capabilities
or supplementary groups. A non-root parent with only bounding capabilities cannot
perform that identity transition and is rejected. Quality mounts the prepared
checkout read-only, with fresh artifact storage and bounded writable scratch.
Preparation uses the official Ayni entrypoint once. Quality activates tooling through
bounded, parsed Mise JSON from protected `/etc/ayni` configuration, restores writable
caller caches, then invokes the fixed Ayni check directly in the prepared checkout.
It does not repeat dependency preparation against the sealed source. Shared
worker sandbox code drops child identity/capabilities, denies network and confines
writes. Parent-owned capture files and preparation evidence remain outside the
repository child identity. The typed Ayni artifact must show the exact managed
lock/certificate/platform, complete locked targets/tools and no command/setup
failure. Legitimate source test/metric failures do not make the runtime incompatible;
their quality result remains separate from environment readiness.

## Publication and recovery

Discovery and publication use `ayni-<arch>-<lock fingerprint>`, for example
`ayni-arm64-<64 lowercase hex characters>`. The fingerprint comes from `.ayni.lock`
without its `sha256:` prefix. Tags contain no operation IDs or orchestration metadata.
A tag locates an image; it is not immutable identity or a registry compare-and-swap
guarantee. Before pushing, the authority retains the intended manifest digest. A lost
response or failed push after intent is `publication_uncertain`; retry reads back
the frozen tag and requires that exact digest rather than rebuilding or spending a
new build allowance. A conflicting digest cannot establish readiness.
A previously ready artifact can be selected by immutable digest, then pulled and
verified again under current inputs. A discovered invalid artifact is never
replaced without both build and replacement grants.

Cancellation retires the bound workload and cleans operation resources. Retry is
available after cleanup and preserves the original frozen inputs, deadline,
build consumption and publication intent. A new allowance or changed selection
requires a new operation. History remains inspectable when configuration changes;
only evidence matching the current selections can become ready.

## Deployment and qualification

The chart defaults runtime preparation off. Enabling it requires immutable worker,
executor and probe image digests, a trusted CLI hash, an explicit `qualified` profile,
resource ceilings, a supervisor authority Secret and immutable signer Secrets in
the dedicated runtime namespace. Signer setup is independently deployment-gated;
it can generate/import those Secrets or an administrator can provision them. See
[image packaging](../../../deploy/images/runtime-preparation/README.md).

The initial profile is native Linux arm64, rootless Podman/vfs in Kubernetes pod
user namespaces (`hostUsers: false`). Actual qualification requires setuid mapping
helpers, unmasked proc, unconfined seccomp and privilege escalation inside that user
namespace. These exceptions apply only to builders. The profile uses no privileged
container, host engine socket, host mount, host PID or SYS_ADMIN capability.
Certifier/finalizer use ordinary restricted non-root pods. Clusters must explicitly
support this profile; there is no Docker-socket or host-execution fallback.

Public build/transfer egress is an explicit deployment choice and NetworkPolicy
excludes private/link-local ranges. This is IP/port enforcement, not DNS-domain
filtering. Credential-free verification has no external egress. PVC requests alone
are not enforced filesystem quotas on Docker Desktop; trusted I/O/expansion limits
and kubelet ephemeral-storage enforcement have distinct roles. A deployment must
qualify capacity for its repositories rather than treating a small smoke build as
proof that every language/toolchain fits.

The packaging README records measured qualification and limits. Live GitHub/GHCR
access, actual publication and repository-specific capacity must be reported as
separate evidence; mocked UI or PostgreSQL tests do not establish them.

### Registry destination

A registry connection selects only `ghcr.io/<organization>`. Admission derives the
lowercase package from the catalog repository name and freezes the complete image
destination before discovery or publication. For `ayni-oss/sandbox`, a connection
at `ghcr.io/ayni-oss` yields `ghcr.io/ayni-oss/sandbox`. Reuse and verification bind
to that complete destination; a connection never supplies a package override.

The Ayni configuration page shows the deployment's runtime profile status,
platform and pinned executor image. Its registry and signing-key defaults
are stored in the current browser. Saved repository selections and retained
runs take precedence over those defaults.

### Local development exception

An operator may explicitly enable `runtimePreparation.localDevelopment: true`
with profile ID `rootless-podman-vfs-userns-local-development-v1` and
`qualified: false` for local development testing. The immutable tooling, source,
signing, allowance, verification and cleanup gates still apply. This exception
permits testing on a cluster whose NetworkPolicy enforcement is not qualified;
it does not establish network isolation. The canonical profile still requires
`qualified: true`, and the development profile rejects that qualification claim.
The frozen operation records this identity and its API exposes `development_only`.
The ayni settings page shows when the active worker profile is for local development.
The guarded local `dev/ctl deploy` and `dev/ctl live` setup selects this exception
automatically after building checksum-verified tooling and resolving immutable
worker/verifier manifests. `AYNI_ORCHESTRATOR_RUNTIME=false` opts out. This local
convenience does not change production chart defaults or qualify the topology.

### Build status report

The build pod reports typed milestones through its bound worker capability.
The console refreshes milestones and elapsed step times for the current build
generation with the run status. The control plane measures elapsed time from
observed milestone boundaries and retains those measurements when it accepts the
build result. A retry resets the report while preserving immutable publication
evidence and the build allowance. Timing is informational and does not affect
stage admission.

### Supervisor diagnostics

A reconciliation failure retries the retained operation and generation; it does
not grant another build or extend its deadline. Supervisor diagnostics identify
failed authority requests by endpoint and HTTP status or response category, and
failed Kubernetes requests by action and API status. Stage failures include the
operation, stage, generation and closed failure reason. Identical reconciliation
failures are logged at most once every 30 seconds. Response bodies, credentials and raw client
errors are never logged. Authority-side store failures include the closed store
reason and SQLSTATE when available.

Accepted stage reports and build checkpoints also leave retained execution
diagnostics. Certification projects an allowlisted cause from its bounded
worker report. The authority stores it with the attempt before the supervisor
deletes the pod. Operators can replay it from the private listener or follow
the dedicated diagnostics stream. See [retained execution diagnostics](../operations/execution-tracing.md#retained-execution-diagnostics).


## Signing key update pull requests

When the selected signing key differs from the repository trust policy, the
onboarding review can open a draft pull request. The control plane uses the
reviewed branch and inspected commit, pins only the selected public key in
`.ayni.toml`, and rebuilds `.ayni.lock` from the verified locked inputs through
Ayni. It aborts if the existing lock cannot be reproduced or the target branch
has moved. The branch is deterministic for the reviewed snapshot and regenerated
lock. Review checks the provider for a draft, open, closed, or merged pull
request on that branch. Retrying the same change returns the existing pull
request, including after its branch is deleted on merge.

The browser remembers integration and lock review checkpoints per repository.
Reopening verifies current integration revisions and resolves the selected
branch before reusing the saved inspection. A changed branch or configuration
returns the wizard to the affected step. Inspections and preparation runs remain
in PostgreSQL, and pull request state is read from the provider. After merge,
check the target branch again before preparing the runtime.

The source-control connection supplies the App credential. The control plane
reads the current installation grants and requests a repository-scoped
Pull requests read token for status. Open PR requests a separate short-lived
installation token with Contents and Pull requests write permissions for the
connected repository. The installation owner must grant both permissions.
Source discovery and worker fetches continue to use Contents read tokens.
