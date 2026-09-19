# Decision 0003: durable local orchestration

Status: accepted, 2026-09-17.

## Decision

M2 uses a single-replica control plane with SQLite on the retained
`control-plane-state` PVC. The database is the authoritative source for task,
attempt, desired-state, idempotency/external-action, worker-image, and
worker-report records.
It is not mounted into workers. Retained managed-check artifacts remain on the
separate `managed-check-evidence` PVC; each artifact directory is bound to an
immutable attempt identity.

The control plane authenticates to the in-cluster Kubernetes API with a narrowly
scoped service account. Its namespace-local Role may create, get, list, and
delete Jobs only. It has no permission to read secrets, exec into pods, manage
PVCs, or access resources in another namespace.

Each attempt owns one deterministic Job name derived from its durable UUID. The
controller follows this state model:

```text
queued -> scheduled -> running -> succeeded | failed
                 \-> interrupted
queued | scheduled | running -> cancellation_requested -> cancelled
```

The Job is created before its `scheduled` state is recorded. If the process
restarts in that interval, creation receives an existing Job and records the
same attempt as scheduled; a second Job is never created. Job success/failure
is observed through the Kubernetes API. A missing non-terminal Job becomes
`interrupted`; it is not retried automatically. A cancellation first changes
durable desired state, then requests deletion of the owned Job, and becomes
`cancelled` only after the Job is absent. Late reports are stored once but cannot
replace a terminal or cancellation state.

Every state-changing API request requires an idempotency key. Submission, retry,
and cancellation keys are durable external-action records. An explicit retry is
permitted only after `failed` or `interrupted` and creates the next task-local
attempt number. The `ORCHESTRATOR_MAX_ATTEMPTS` setting is validated between one
and ten (three by default), then persisted with each task so a later deployment
configuration change cannot relax an existing task's retry bound. M2 has no
provider actions yet, but this same table is the recovery boundary for M3
draft-PR creation.

## Why

SQLite plus a PVC is the smallest real durable store for the one-node private
Docker Desktop MVP. It permits deployment replacement and process restart tests
without introducing a separate database service before the product needs one.
Separating task state from managed evidence prevents a disposable or
agent-controlled workspace from becoming the record of truth.

The application API remains authoritative, as specified in
[Decision 0002](0002-product-name-and-control-plane-model.md). M2 therefore
uses ordinary Jobs rather than CRDs. A deterministic name and durable desired
state let reconciliation recover from duplicate scheduling calls and uncertain
Kubernetes responses without treating the Kubernetes object as the product's
source of truth.

## Consequences and limits

- This profile intentionally has one control-plane replica. Multi-replica
  operation requires a shared transactional store and leader/reconciliation
  coordination; it is not implied by a PVC.
- The configured worker image is supplied by the local build/load workflow and
  recorded in the control-plane configuration. The controller does not accept a
  repository-supplied image reference.
- Retained evidence uses the existing M1 worker contract. M2 records attempt
  identity and Job observation; it does not loosen Ayni lock, executor, artifact,
  or final-validation requirements.
- Normal cleanup retains both PVCs. Deleting retained task state or evidence is a
  separate data-retention decision.
- M2 does not select an agent, provider credential, conversation checkpoint, or
  draft-PR action. Those decisions remain M3 work.

Revisit this decision when multiple control-plane replicas, a non-local
deployment profile, artifact-object storage, or a CRD-authoritative user
experience becomes a tested requirement.
