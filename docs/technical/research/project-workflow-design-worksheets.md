# Project workflow design worksheets

Date: 2026-10-19. Status: research templates, not approved contracts.

Use these worksheets with the [refactor runbook](project-workflow-refactor-runbook.md). Fill them with evidence during research; do not treat illustrative names as an agreed schema. Delivery status belongs in issues.

## Product and domain model

For Workspace, Project, Conversation, Plan, Plan Version, Task, Attempt, Runtime, Session, and Recovery Proposal, record:

| Field | Required answer |
| --- | --- |
| Meaning | One sentence meaningful to the engineer using the product. |
| Ownership | Parent identity, authoritative writer, and lifecycle owner. |
| Relationships | Cardinality, references, and behavior when another object changes or disappears. |
| State | Allowed transitions, terminal conditions, and optimistic concurrency rules. |
| History | Immutable content, revisable content, provenance, and retention. |
| Access | Existing scope boundary; workspace sharing must not imply sharing private project data. |
| Presentation | What the engineer sees, can act on, and can recover. |

Resolve whether plans belong to projects independently of conversation origin. Distinguish a plan version from any execution of it, and a task from its physical attempt. Distinguish runtime configuration, certified image revision, and running workload.

Separate project facts, decisions, interpretations, and questions. Define how accepted task outcomes reach all relevant conversations, and how stale agent assessments are rejected. Decide whether one project coordinator serializes assessments or concurrent sessions propose version-checked changes.

## Current-system evidence

Trace message submission, plan approval, worker progress, failed task recovery, runtime preparation, and browser reconnect. For each, collect the entry point, persistence transaction, scheduler, producer, consumer, authorization check, and UI update path.

Inspect Cargo and package dependencies, protobuf imports, `.ayni.toml` forbidden edges, database ownership, Kubernetes permissions, and event subjects. Read tests as evidence of assertions, not proof of deployed behavior. Record source revision, test command, fixture limitations, and observed timings.

Produce two maps: logical ownership and physical deployment. Mark synchronous dependencies, durable delivery, shared storage, and external side effects separately. Identify circular dependencies, hidden database coupling, and behavior duplicated across boundaries.

## Task feedback and recovery

Model blocked-for-input separately from infrastructure interruption, failed execution, failed quality, and failed publication. Determine which cases preserve a live session and which require a checkpoint or new attempt.

A recovery proposal must identify the task, attempt, failure or question, retained artifact/checkpoint digest, original plan version, proposed changes, budget, and expected revisions. Display what will repeat and which external effects already occurred. Confirmation must bind the exact proposal and produce an idempotent receipt.

| Case | Decision and evidence required |
| --- | --- |
| Safe checkpoint exists | Prove source, runtime, consumed budget, phase, and writer identity before continuing. |
| Inputs or scope change | Decide whether a new plan approval or attempt is required; preserve the original frozen inputs. |
| Checkpoint is missing or unsafe | Explain why resume is unavailable and propose an explicit alternative. |
| Publication already started | Reconcile remote effects before retrying; prevent duplicate push or PR creation. |
| Answer becomes stale | Revalidate proposal against cancellation, task progress, and newer decisions. |
| Two replies or approvals race | Retain one accepted resolution and explain the conflicting decision. |

Questions and failures appear in the originating conversation and project activity. Define one durable attention/resolution identity so these views cannot disagree. A human answer supplies context; only confirmed recovery authorizes execution.

## Events and JetStream

Start with semantic families, then choose physical streams and subjects. Commands request an action; reports describe producer observations; accepted facts record authoritative decisions. Progress and diagnostics have different retention and privacy needs.

For every proposed message, complete this contract:

| Field | Required answer |
| --- | --- |
| Identity | Stable event ID, type, schema version, producer identity, and scope. |
| Correlation | Project, conversation origin, plan version, task, attempt, causation, and trace references where applicable. |
| Authority | Who may publish, what the message proves, and who may accept it. |
| Ordering | Aggregate revision, writer generation, sequence, and treatment of late events. |
| Payload | Bounded safe fields, evidence references and digests; exclude secrets and unrestricted logs. |
| Delivery | Retry, deduplication, acknowledgement boundary, poison-message handling, and recovery. |
| Retention | Replay horizon, durable record location, deletion policy, and reconstruction behavior. |
| Evolution | Version compatibility, unsupported-version handling, and producer/consumer rollout. |

Evaluate separate lanes for execution commands, worker reports, accepted domain facts, and transient activity. Do not assume each family needs its own physical stream. Compare retention, consumer isolation, access control, throughput, and ordering requirements before assigning subjects.

Resolve producer durability: an outbox or retained spool must cover crashes between a local result and publication. Resolve consumer atomicity: durable processing and deduplication precede acknowledgement. Broker acknowledgement is not evidence acceptance or permission for a side effect.

Specify bounded broker credentials, subject permissions, and authenticated producer-to-execution bindings. Repository code and model tools must not impersonate approvals or accepted outcomes. Avoid relying on subject names alone as authorization.

Define project-agent consumption independently of pod lifetime: cursor ownership, wake policy, backlog compaction, turn serialization, coalescing, and budget. Progress may update the UI without invoking the model. Replay must rebuild context without repeating paid calls or external effects.

## gRPC and browser contracts

Inventory existing services before proposing replacements. For each RPC, record caller, owner, scope, request, response, deadline, cancellation, idempotency, expected revision, errors, and compatibility decision.

Compare unary durable commands with server streams for observation. Evaluate internal worker/session streaming separately from the browser transport. Persist admission before acknowledging a command; disconnecting an observation stream must not implicitly cancel its task.

For each watch, specify initial snapshot, snapshot boundary, cursor scope, ordering, replay, cursor expiry, backpressure, heartbeat, and authorization. For a project-wide view, define how updates from multiple owners form a consistent projection. Independent per-resource cursors cannot be silently treated as one global cursor.

Prototype gRPC internally and compare SSE with a browser-compatible gRPC transport. Measure update latency, reconnect behavior, proxy constraints, and implementation complexity. Keep editor state separate from streamed read state whichever transport wins.

## Component and dependency decisions

Evaluate logical owners before assigning crates, packages, or deployments. Candidate responsibilities include workspace policy, project knowledge, planning, execution coordination, runtime management, activity projections, and provider integration.

| Boundary | Use when | Required evidence |
| --- | --- | --- |
| Module | One owner encapsulates policy within a process. | State ownership, interface, and enforced dependency direction. |
| Library | Multiple owners share a stable contract or mechanism. | Real consumers; no hidden application policy or shared mutable authority. |
| Service | Independent deployment, scaling, availability, or security isolation is needed. | Operational benefit, failure contract, data ownership, and deployment cost. |
| Worker/session | Work needs disposable execution, scoped credentials, or resource isolation. | Lifecycle, admission, fencing, recovery, and reporting contract. |

For each proposed component, list owned state, inbound/outbound contracts, permitted dependencies, external actions, and failure behavior. Name the authority for every transaction. If components share tables, document that coupling rather than claiming independent service ownership.

Compare at least a modular control plane with workers against a split coordination/projection deployment. Evaluate storage choices against write concurrency, transaction boundaries, recovery, and operational scope. Keep deployment tenancy expansion outside this exercise.

## Feasibility and acceptance evidence

Use deterministic fixtures for policy and real local infrastructure for transport and recovery. Each experiment needs a hypothesis, reproduction command, expected observation, and a decision affected by the result.

- Crash producers before and after publication; crash consumers before commit and before acknowledgement.
- Duplicate and reorder progress, completion, approval, and cancellation messages.
- Expire a planner session while events arrive; verify catch-up without duplicate reasoning or actions.
- Confirm recovery twice; verify a single execution and retained original evidence.
- Lose the broker or database; verify bounded queues, visible degradation, and eventual convergence.
- Inject a slow conversation among many active conversations; measure scheduler fairness and queue age.
- Reconnect streams with valid and expired cursors while editing; verify no lost accepted activity or overwritten drafts.
- Exercise a real task checkpoint and publication recovery on Docker Desktop Kubernetes; distinguish fixture evidence from provider execution.

## Decisions and implementation handoff

For each decision, record alternatives, chosen contract, evidence, costs, unresolved risks, and reversal conditions. Keep current implementation, target proposal, and approved decision visibly distinct.

Only then map current components to keep, replace, adapt, or retire. Each delivery issue needs prerequisites, an observable outcome, contracts changed, failure tests, required local checks, and removal work. Estimate effort after the high-risk experiments, with assumptions and confidence rather than a percentage-complete claim.
