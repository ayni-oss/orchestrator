# Decision 0004: pilot readiness and reusable preparation

- Status: accepted for M4 validation
- Date: 2026-09-25

## Context

M3 proved one bounded API request through coding, independent validation, and an
exactly-once draft PR. M4 needs a maintainer interface, actionable repository
readiness, and measurable environment reuse without letting editable agent state
become validation authority.

## Decision

The selected pilot remains one trusted Docker Desktop cluster and the allowlisted
`ayni-oss/sandbox` repository. The control plane serves a small browser client
from its existing loopback-forwarded Service. SQLite remains the durable source
of lifecycle truth; the UI only calls idempotent JSON actions.

Repository readiness is an explicit durable preparation lifecycle. A
preparation identity binds repository enrollment, exact revision, lock digest,
Ayni `0.13.0`, and recipe schema. A cold Kubernetes Job checks short-lived
GitHub App access, checks out the immutable revision, verifies the committed
Ayni contract, builds and doctors the environment in an isolated engine, and
retains:

- the Ayni build record;
- the exact environment image archive;
- a digest-bound preparation manifest and cold duration.

The retained preparation claim is separate from task checkpoints and validation
evidence. A compatible repeat uses the ready identity without a new Job and
records reuse. Validation hashes the archive and build record, loads the exact
image into a fresh isolated engine, and requires `ayni env doctor` before
running the managed check. A task freezes the preparation identity. Changed
revision or lock cannot use an older preparation; failed replacement remains a
visible failure and may be explicitly retried.

Retry and requested-input actions create linked new tasks. They do not alter an
old task, checkpoint, validation result, or PR. A coding agent can request one
concrete missing input through a bounded checkpoint file; the maintainer's input
is appended to the linked task request.

Dependency evolution is **unsupported** in this pilot. Candidate changes to the
Ayni lock or Rust dependency/toolchain manifests return
`dependency_change_unsupported` and never publish. Environment/dependency updates
must be reviewed outside this workflow, enrolled with the new lock, and prepared
before feature work. Protected `.ayni.toml` changes remain `policy_rejected`.

The unauthenticated pilot UI is reachable only through loopback port-forwarding
inside a trusted local cluster. One control-plane replica, privileged DinD,
namespace-scoped Secret management, no automatic retention expiry, and fixed
agent/Job/storage budgets are declared operating limits rather than production
properties.

## Consequences

Warm validation avoids the environment build while still uses a fresh engine and
fresh candidate checkout. The archive consumes retained disk and loading it is
not free; M4 records cold preparation duration and reuse identity, not a claim of
zero warm latency. Single-node `ReadWriteOnce` sharing is topology-specific.

A controller or PVC compromise remains inside the trusted local administrative
boundary. Multi-user authentication, multi-node storage, horizontal replicas,
network policy, general repository onboarding, dependency rebuilding, automated
retention, and production secret management are deferred.

Normal cleanup must never delete retained claims. Updating applies additive
SQLite migrations and creates the preparation claim in place. Destructive PVC
removal requires a separate explicit operator command after backup.
