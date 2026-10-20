---
layout: home
hero:
  name: Ayni Orchestrator
  text: Durable workflow foundations
  tagline: M1 Slice 1 is a contributor preview of PostgreSQL-authoritative hierarchy, message admission, snapshots, and history.
  actions:
    - theme: brand
      text: View source
      link: https://github.com/ayni-oss/orchestrator
    - theme: alt
      text: Read the milestone
      link: https://github.com/ayni-oss/orchestrator/blob/alpha/docs/technical/research/project-workflow-m1-durable-foundation.md
features:
  - title: PostgreSQL authority
    details: Forward migrations, scoped idempotency, FIFO admission, exact outbox bytes, and bounded projections share one durable transaction boundary.
  - title: Private by construction
    details: Canonical workflow gRPC binds to pod loopback and is available only through an explicit trusted diagnostic port-forward.
  - title: Honest slice boundary
    details: Execution, resumable watches, providers, browser UX, and custom-client workflow access are deliberately deferred to later slices.
---

## Contributor preview

The prior browser and SQLite workflow were removed in a clean cutover. The current runnable slice is intended for contributors validating the replacement foundation on Docker Desktop Kubernetes. It is not an adopted-user migration or shared-environment release.

Current evidence covers PostgreSQL 17.6 migrations, concurrent startup, rollback, scoped idempotency, FIFO admission, queue bounds, snapshots/history, readiness, canonical private-v1 mapping, and retained local storage.

See the repository's [technical documentation](https://github.com/ayni-oss/orchestrator/tree/alpha/docs/technical) for architecture and local operations.
