# Durable task event direction

Status: planned alpha direction. Conversation execution already uses retained
JetStream commands/reports/decisions. Coding tasks currently use authenticated
HTTP callbacks and a polling supervisor; this document does not describe a
completed transport cutover. Product scope is defined by the
[alpha contract](alpha-product.md); delivery belongs to
[roadmap milestones C and D](../../../ROADMAP.md#c-autonomous-task-to-reviewable-pr).

## Communication responsibilities

| Traffic | Planned boundary |
| --- | --- |
| Task dispatch, cancellation, terminal reports, authorization and acceptance decisions | Durable JetStream delivery backed by transactional intent and retained consumer state. |
| User commands, snapshots and bounded queries | Canonical gRPC, with the existing local browser adapter. |
| Provisional assistant text and tool progress | Bounded transient streaming, with scoped accepted-state reconciliation after reconnect. Core NATS is an option, not a required durable token log. |
| Sources, patches and large quality artifacts | Immutable artifact references carrying digest, size, owner and access scope. |
| Kubernetes workload changes | Watches plus bounded periodic reconciliation, exact workload adoption and cleanup confirmation. |

Keep PostgreSQL as the transactional authority. Extend existing publication and
report acceptance mechanisms rather than creating a second event ownership
framework. Define command, observation and accepted-decision semantics explicitly;
not every message is an accepted domain fact.

## Proposed task handoff

1. Admission commits frozen execution intent and exact command/outbox bytes atomically.
2. A trusted supervisor consumes available work within its capacity, retains assignment and reconciles the exact Pod identity.
3. Workload binding and provider/tool authorization requests receive explicit application decisions before the protected action proceeds.
4. A trusted communication process outside the coding process retains reports and delivers immutable evidence references.
5. The control plane validates scope, generation, Pod identity, allowance and evidence, then commits acceptance/rejection and any outgoing decisions before acknowledging delivery.
6. Accepted outcomes update task/DAG/project projections; consumers release retained handoff state only after the matching application decision.

Message names, subjects, stream topology and the choice of sidecar versus another
trusted local process need a focused protocol design before implementation. Keep
the canonical v1 contracts, producer/consumer ACLs, retention and capacity limits
explicit. Broad broker credentials never enter the agent's command environment.

Every envelope binds stable identity, workspace/project/task/attempt, generation,
causation/correlation, relevant command/evidence digests and scoped destination.
Redelivery reuses identity and exact bytes. Duplicate or stale reports retain a
decision without authorizing another attempt or overwriting newer accepted state.

## Authority, recovery and backpressure

Receiving a dispatch message does not itself authorize a paid model call, MCP
mutation, runtime build or PR creation. Review may grant bounded work in advance;
the authoritative permit still binds the exact admitted scope and action. An
uncertain response remains visible and cannot trigger blind external replay.

Cancellation fences accepted ownership, reaches the responsible supervisor and
retains stop/cleanup acknowledgement. Reconnecting owners recover unresolved
assignments and decisions; workload absence is not proof that an external action
never happened. Independent quality stages retain separate trusted evidence and
credential boundaries.

Bound queued work, pulls, in-flight reports, artifact reads, Pod creation and
provider calls. Replace hardcoded capacity assumptions with explicit measured
limits and fair allocation as required by the product workload. A broker outage
leaves committed work pending with visible backpressure; it must not lose accepted
intent or allow unbounded local buffering.

## Incremental cutover and acceptance

Ship dispatch/assignment first, then protected-action decisions and reports, then
cancellation/recovery and DAG release. Every slice runs an actual task with its UI
and failure cases. Define one active owner per attempt during cutover; do not
introduce dual execution through polling and broker delivery. Remove a replaced
active path after equivalent acceptance, retaining historical evidence in Git.

Required local scenarios include duplicate dispatch, crash before/after publication,
report acceptance and ACK loss, stale generations, unknown provider outcomes,
broker outage, workload replacement, cancellation/cleanup and artifact denial or
digest mismatch. Prove downstream release once from accepted outcomes and no
release from provisional or rejected observations.

Measure queue age, admission-to-start latency, outbox backlog, report acceptance,
redelivery, cancellation-to-stop and resource/usage bounds. Configurable limits
and safe multi-supervisor claims prepare for growth; thousands of agents require
separate sustained-load evidence. Transport replacement alone cannot establish
that capacity.

## Current implementation references

- [Conversation broker](../../../backend/servers/control-plane/src/broker.rs) owns exact PostgreSQL outbox publication and report acceptance delivery.
- [Task supervisor](../../../backend/workers/conversation-supervisor/src/bin/ayni-task-supervisor.rs) currently polls the task authority and reconciles workloads.
- [Task authority](../../../backend/servers/control-plane/src/task_authority.rs) currently accepts authenticated worker HTTP requests.
- [Task isolation](isolated-task-execution.md), [publication](task-publication.md) and [recovery](task-recovery.md) describe current retained authority that the event cutover must preserve.
