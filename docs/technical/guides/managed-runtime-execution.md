# Managed runtime execution

The managed profile binds a connected repository, committed lock, verified
runtime artifact and digest. Reusable App credentials stay inside the connector
boundary. Each worker receives short-lived repository-scoped credentials for
checkout, push and draft creation. The control plane refreshes attempt
credentials while the worker is active.

The worker attests its runtime before checkout, verifies source and lock, codes,
runs mandatory full checks, repairs within frozen limits, binds final evidence
and publishes a truthful draft. The control plane only schedules, observes,
collects and cleans up task execution. Environment build Jobs remain independent
of task execution and retain their narrowly scoped build authority.

The Helm runtime configuration exposes `codingTurns: 24`,
`qualityRepairLimit: 3`, concurrency and runtime policy. Repair limits range from
0 to 10. Changes to deployment defaults affect newly admitted tasks; frozen
attempt limits remain unchanged after restart.

Ready runtimes are re-verified hourly by default (`runtime.intervalSeconds: 3600`
in Helm, `interval_seconds` in operator JSON), plus per-profile scheduling jitter.
Initial preparation and explicit Retry or Rebuild requests do not wait for this
periodic interval. In-progress operations continue to be observed independently.

Local execution uses the Docker Desktop umbrella chart described in
[local development](../operations/local-development.md). Production retains platform node
selection, digest pinning and resource limits; emulation is a local policy.

## Worker sandbox

The Linux worker requires fully enforced Landlock ABI 6 (kernel 6.12 or newer).
Both agent shell commands and Ayni quality execution run inside the same
unprivileged sandbox because repository tests are untrusted code. System and
toolchain paths are read-only; the workspace and disposable scratch paths are
writable. Worker output, repository/model credentials, frozen requests and the
launcher remain outside that authority. Signal and abstract Unix-socket scopes
prevent sandboxed children from signaling the parent or reaching its abstract
sockets. Unsupported or partial enforcement fails closed: there is no
unrestricted fallback. This needs no privileged container, added capabilities,
or special Docker/Kubernetes privilege.


## Build, certify, publish and verify

Before scheduling, SQLite persists an operation identity and frozen source commit,
selected runtime revision, complete lock fingerprint/digest, platform, certificate
revision, destination and immutable credential Secret references. Idempotent Job
names adopt existing resources after controller restart. Exact source checkout
uses repository-scoped access confined to the checkout init container.

The build Job invokes the pinned `ayni env build --executor-image <digest> --tag <candidate>` contract. Ayni validates committed policy and lock consistency,
prepares dependencies and certifies protected environment content. The orchestrator
checks the frozen envelope and resulting local image identity/platform; it does
not implement Ayni policy evaluation. Certification failure stops publication.
This phase does not establish editable-source code quality.

A separate trusted publication Job rechecks the discovery tag, rejects a different
candidate unless an explicit Rebuild authorized replacement, then tags the certified local image ID with the
frozen destination and pushes it. It executes no repository code and receives
push access and engine TLS credentials, with neither source nor signing access.
The builder receives signing and engine authority with pull-only registry access;
repository dependency execution inside the isolated engine receives neither
signing nor push credentials. Engine clients run in trusted orchestrator code.

Registry lookup and verification receive pull-only access. Discovery resolves the
tag, selects the requested platform from an index and validates the manifest
hash. Verification runs the pinned worker against the immutable digest, validates
lock, selected certificate, trusted signed runtime metadata and protected content
using supported Ayni contracts. Quality results are separate and do not gate image
certification. Activation compares frozen source/configuration and selected revision
again. Only a matching verified artifact becomes Ready; task admission freezes it.

The only image distribution mechanism is registry push/pull. Kubelet owns normal
node image caches and garbage collection. No workload receives a Docker/containerd
node socket or hostPath; no image artifact PVC exists. A task-output PVC retains
coding work and evidence, independently of image preparation.

## Naming

The centralized destination is `<registry>/<owner>/<repo>:ayni-<arch>-<lock>`.
`arch` is `amd64` or `arm64` from Linux execution policy. Only the validated lock's
`sha256:` prefix is removed; all 64 lowercase hexadecimal characters remain.
OCI digest references keep `@sha256:`. For example:

```text
ghcr.io/ayni-oss/sandbox:ayni-arm64-717bea5f26efff5396bd50dbd1c9efa4a36f3ffc9f6a77cda9661d50e401a3e5
```

The ARM64 and AMD64 tags are 75 characters, below the 128-character tag bound.
Repository paths are limited to 255 characters and complete references to 512.
Invalid names fail instead of truncating identity. Images use the source
repository owner and name. The selected connection supplies credentials with
access to that destination. GHCR is the currently supported registry provider.

## Failure and recovery

Only a manifest 404 after authenticated discovery authorizes an automatic missing
image build. Authentication errors, rate limits, outages and wrong platform are
findings. Existing invalid certification or content requires an explicit Rebuild.
A Rebuild freezes current inputs into a new intent; it cannot bypass setup.

Discovery reads its pull-only credentials from a read-only Kubernetes Secret
projection, including Kubernetes' symlinked keys, with a 64 KiB size bound.
Repository-controlled input files continue to reject symlinks. A credential-read
failure does not establish image absence or authorize a build.

After publication finishes or fails, reconciliation inspects the registry before
retrying. A published candidate proceeds directly to independent verification.
An interrupted or missing publication Job also triggers registry inspection before
its push can be reissued. If still absent, publication retries the same certified local image ID. If that
candidate is evicted, the isolated builder rebuilds frozen inputs under a new Job
identity. No unpublished-image retention is promised. Publication attempts are
bounded at three, with registry readback before any repeated push. Failed build
and verification executions are counted once per persisted Job identity, stopping
after three until explicit Retry or Rebuild. Observation, reporting and cleanup
errors back off independently and retain the same execution. Repository analysis
does not replenish retries. Changed frozen preparation inputs admit a new budget;
execution history survives both this change and explicit retry. See the
[retry and reporting behavior](../architecture/runtime-runner.md) for migration and recovery details.

Owned Jobs, temporary checkout/configuration resources and transient credentials
are cleaned after persisted outcomes. Exhausted operations retain bounded outcome metadata for diagnostics; their Jobs
and transient credentials are cleaned, and restart recovery cannot recreate them
until an explicit retry or Rebuild. Cleanup never deletes
published images or purges node caches. Registry retention is operator policy.

Public certificate trust preparation runs in the pinned worker image with Ayni and
the pinned lock resolver. It needs checkout access and the public key only, and
proposes owner-reviewed policy/lock trust updates using the catalog-selected
official builder and public certificate data. Missing engine infrastructure blocks
subsequent runtime builds. Trust updates reuse `setup/certificate-trust` within each
repository. An existing open managed proposal (including an earlier generated
branch) is updated rather than duplicated. Updates preserve default-branch and
proposal ancestry and refuse to overwrite unrelated branch changes. Closed
unmerged proposals require an explicit retry to reopen; merged proposals retire
the review link. Later required trust changes open a new PR on the same branch,
since GitHub cannot reopen a merged PR.

See [repository-selected official builders](../reference/repository-builders.md) for immutable
version selection, attempt engines, security modes and recovery.
