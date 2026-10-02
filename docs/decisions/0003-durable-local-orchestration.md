# Decision 0003: durable local orchestration

- Status: superseded by [Decision 0007](0007-clean-v1-runtime-profiles.md)
- Date: 2026-09-17

## Historical scope

This superseded decision describes the retired pilot's preparation and
worker-report storage. In particular, the `managed-check-evidence` claim is not
part of the clean-v1 deployment; assessment evidence is introduced only with
the trusted collector and durable assessment contract.

## Decision

Use a single-replica control plane with SQLite on the retained
`control-plane-state` persistent volume claim. The database is authoritative for
tasks, attempts, desired state, idempotency, external actions, worker identity,
preparations, and worker reports. Workers never mount the database.

Keep managed Ayni artifacts on the separate `managed-check-evidence` claim and
bind each artifact directory to an immutable attempt identity.

The control plane uses a namespace-scoped Kubernetes service account. Base
orchestration grants only the Job operations required for reconciliation.
Development publication adds separately documented local-profile permissions.

## Attempt reconciliation

Each attempt owns one deterministic Job name derived from its durable UUID.

```text
queued -> scheduled -> running -> succeeded | failed
                 \-> interrupted
queued | scheduled | running -> cancellation_requested -> cancelled
```

The Job is created before `scheduled` is recorded. If the process restarts in
that interval, a repeated create observes the existing Job and records the same
attempt. A missing non-terminal Job becomes `interrupted`; it is not retried
automatically. Cancellation records durable intent before deleting the owned
Job. Late reports are retained once but cannot replace a terminal or cancelled
state.

Every state-changing request requires an idempotency key. An explicit retry is
allowed only after a failed or interrupted attempt and creates the next
attempt. The validated maximum-attempt bound is persisted with the task so a
later deployment setting cannot expand it.

The same durable external-action model protects GitHub publication. After an
uncertain response, reconciliation checks the deterministic provider state
before retrying creation.

## Rationale

SQLite plus a retained volume is the smallest real durable store for the
single-node Docker Desktop profile. It supports process and Deployment
replacement without introducing a separate database service. Deterministic Job
identity and durable desired state absorb duplicate scheduling and uncertain
Kubernetes responses without making Job status the product record.

## Consequences and limits

- The control plane must remain single-replica with this storage model.
- Multi-replica operation requires a shared transactional store and coordinated
  reconciliation ownership.
- Worker images are selected by trusted deployment configuration, never by a
  repository.
- Normal cleanup retains task state and result storage.
- Deleting retained state is an explicit data-retention decision.
- A non-local topology, object artifact storage, or CRD-authoritative interface
  requires revisiting this decision.
