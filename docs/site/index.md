---
layout: home
hero:
  name: Ayni Orchestrator
  text: Durable workflow foundations
  tagline: A contributor preview of isolated conversations, opt-in OpenAI responses, scoped read tools and resumable updates.
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
  - title: Durable handoff
    details: Authenticated JetStream and retained conversation sessions keep broker storage, ownership claims, decisions, and accepted results distinct.
  - title: Private by construction
    details: Canonical workflow gRPC binds to pod loopback. Contributors can resume updates; the local contributor console supports scripted and explicitly enabled OpenAI conversations, reported usage, cancellation and explicit uncertainty acknowledgement. Shared access remains deferred.
---

## Contributor preview

The prior browser and SQLite workflow were removed in a clean cutover. The current runnable slice is intended for contributors validating the replacement foundation on Docker Desktop Kubernetes. It is not an adopted-user migration or shared-environment release.

Current evidence covers PostgreSQL 17.6 migrations, scoped idempotency, FIFO admission and promotion, authenticated retained streams, signed executor ownership, accepted scripted responses, snapshots/history, restart recovery, and the private-v1 boundary.

See the repository's [technical documentation](https://github.com/ayni-oss/orchestrator/tree/alpha/docs/technical) for architecture and local operations.
