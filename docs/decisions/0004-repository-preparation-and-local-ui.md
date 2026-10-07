# Decision 0004: repository preparation and local UI

> Historical record: task-pipeline and quality/publication ownership in this decision are superseded by the [worker-owned architecture](../architecture.md). They do not require compatibility code.
- Status: superseded by [Decision 0007](0007-clean-v1-runtime-profiles.md)
- Date: 2026-09-25

## Historical scope

This superseded decision records the retired pilot preparation model only. Its
Docker-in-Docker, build-record, archive, `env doctor`, and validation-manifest
requirements are not current instructions. The released Ayni 0.14 integration
contract is recorded in [the current integration contract](../ayni-0.14-integration.md).

## Decision

The current profile supports one trusted Docker Desktop cluster and GitHub.com
repositories explicitly selected for its configured GitHub App. The browser remains reachable only through loopback port-forwarding. The
initial embedded client and JSON actions were replaced by the SvelteKit
server-side BFF described in Decision 0005. SQLite remains the durable lifecycle
source of truth.

Repository readiness is an explicit preparation lifecycle. A preparation
identity binds repository enrollment, exact revision, lock digest, supported
Ayni version, platform, and recipe schema. A cold Kubernetes Job validates
short-lived GitHub App access, checks out the immutable revision, verifies the
committed Ayni contract, builds and doctors the environment in an isolated
engine, and retains:

- the Ayni build record;
- the exact environment image archive; and
- a digest-bound preparation manifest and cold duration.

A compatible repeat reuses that identity without a new build and records reuse.
Final validation verifies the archive and build record, loads the exact image
into a fresh isolated engine, and runs `ayni env doctor` before the managed
check. A task freezes its preparation identity. Changed inputs cannot use an
older preparation, and a failed replacement remains visible until explicitly
retried.

Retry and requested-input actions create linked tasks. They never mutate an old
task, checkpoint, validation result, or pull request.

Dependency-manifest evolution is unsupported by the current profile. Changes to
the Ayni lock or Rust dependency/toolchain manifests return
`dependency_change_unsupported` and do not publish. Protected Ayni policy changes
return `policy_rejected`. Each task also freezes normalized protected paths and
whether test edits are authorized. Final validation compares the complete diff
and supported Rust test syntax with the starting revision; violations return
`scope_rejected` and cannot publish.

## Access and operating limits

The browser/API has no end-user authentication. It must not be exposed through
Ingress, NodePort, or a non-loopback forward. One control-plane replica,
privileged Docker-in-Docker engines, namespace-scoped Secret management, fixed
agent/Job/storage budgets, and no automatic retained-data expiry are explicit
local trusted-team constraints rather than production properties.

## Consequences

- Warm reuse avoids rebuilding but still performs fresh candidate validation.
- Prepared archives consume retained disk and need explicit backup and retention
  policy.
- Single-node `ReadWriteOnce` volume sharing is topology-specific.
- Normal cleanup preserves every retained claim.
- Updating uses additive SQLite migrations; destructive volume removal requires
  an explicit operator command after backup.
- Multi-user authentication, multi-node storage, horizontal replicas, network
  policy, GitLab or other VCS onboarding, dependency rebuilding, automated
  retention, and production secret management are not supported by this profile.
