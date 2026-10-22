# M3 delivery plan

Date: 2026-10-21. Status: staged implementation plan. PR #238 combines this contract with the first durable-review slice; the second slice adds structured agent proposals; the third slice adds exact-version approval and atomic task admission; final cumulative acceptance remains planned.

## What this decides

- Implement the [plans and approval contract](../architecture/plans-and-approval.md) through four cumulative, reviewable PRs targeting `alpha`.
- Each PR starts from current `origin/alpha` after its prerequisite merges.
- Extend the existing control plane, private-v1 contracts and cumulative acceptance harness.
- Preserve task origin from admission through the future M4/M6/M7 boundaries.

## Why now

M2 is complete on `alpha` at `8e6381b`, following PRs #234–#237.
Plans and task approval are the next [roadmap scope](../../../ROADMAP.md#delivery-sequence).

## How it works today

- Canonical protobufs define conversation, configuration, session and provider operations.
- `workflow-postgres` owns migrations, receipts, projections and application acceptance.
- The executor retains provider results and signed report bytes; the console owns no domain authority.
- The [conversation acceptance matrix](../operations/conversation-acceptance.md) supplies cumulative recovery gates.

## The proposal

### Code ownership

| Existing boundary | M3 extension |
| --- | --- |
| `proto/ayni/orchestrator/v1` and generated bindings | Plan/version reads, mutations, explicit approval, task projections and typed errors within v1. |
| `proto/ayni/orchestrator/events/v1`, `backend/libs/events` | Bounded proposal report/decision payloads, origin correlation and authenticated producer validation. |
| `backend/servers/control-plane/crates/workflow-postgres` | Forward migrations, constrained origin references, immutable versions, approval uniqueness, admission transactions and replay projections. |
| `backend/servers/control-plane` | Transport validation, operator commands, signed proposal acceptance and console allowlist extensions. |
| `backend/libs/providers`, `backend/workers/scripted-executor` | Bounded structured proposals, retained output, signed handoff and restart/cancellation fencing. |
| `apps/console` | Plan review, version comparison, explicit approval and admitted task detail with visible project/conversation origin. |
| `backend/tests/workflow-acceptance`, `scripts/ci`, `dev/fixtures/provider` | Integrated proposal, approval, observation and interruption cases using existing local topology. |

Do not restore superseded workflow implementations or introduce parallel interface versions.
Keep transport-independent persistence and executor credential boundaries intact.

### Contract completion before code expansion

The first runtime PR records exact message schemas and limits before adding tables or endpoints.
These are implementation decisions constrained by the binding contract, not permission for downstream scope expansion.

| Decision | Required concrete specification |
| --- | --- |
| Canonical content | Exact fields, normalization, digest bytes, version-local task keys and schema validation. |
| Bounds | Maximum plan bytes, tasks, field sizes, versions, references, list pages and retained proposal capacity. Never silently truncate. |
| State and errors | Current-version concurrency, one approval per plan, stale/conflict/capacity errors, unresolved-input eligibility and receipt replay rules. |
| Commands and reads | Exact canonical v1 methods, request/receipt fields, cursor rules and transaction boundaries. |
| Proposal path | Structured provider representation, invocation/result identity, signed report binding, deduplication and retained-result replay. No model approval capability. |
| Execution handoff | Declared versus verified repository/source inputs, execution limits and visible blockers; M3 admits work without declaring it executable. |
| Retention | Protected origin/version/receipt references, quotas and rejection before acceptance when retained capacity is exhausted. |

Any change to a binding product rule updates the contract and roadmap in the same PR.

## The plan

| PR | Prerequisite and observable outcome | Acceptance before merge |
| --- | --- | --- |
| 1: Durable plan review | M2. Specify exact contracts, add persistence and operator-authored version creation, bounded reads and console review. Every version retains ownership; editing creates a new immutable version. | Create/revise/reload against real PostgreSQL and the deployed console. Reject cross-scope references, stale edits, invalid content and excess bounds. Reconnect preserves accepted versions and unsaved local edits. |
| 2: Agent plan proposals | PR 1. Discuss scope and constraints, then accept a bounded structured provider proposal into the originating conversation. Show assumptions, questions and evidence provenance. | Deterministic malformed and valid proposal fixtures; retained result replay; stale/cancelled generation rejection; no cross-conversation writes or approval authority. A separately reported bounded live-provider smoke exercises real proposal creation. |
| 3: Approval and task admission | PR 2. Review exact version/digest, explicitly approve, and atomically retain one receipt plus the complete task set. Display immutable task origin and awaiting-execution state. | Race approval against revision and two-tab approval. Duplicate keys and different-key races cannot duplicate admission. Wrong scope/digest and unresolved scope fail. Restart and ambiguous retries return original identities. No coding dispatch occurs. |
| 4: Cumulative M3 acceptance | PR 3. Close remaining failure cases and document deployed evidence, operations and the M4/M6/M7 handoff. | All matrix cases below pass in the cumulative harness; affected behavior runs on explicit Docker Desktop Kubernetes. Delivery PR records actual source/image identities, fixture/live distinctions and failed or blocked checks. |

Each PR extends the harness immediately; PR 4 does not defer correctness or failure tests from earlier slices.
Update roadmap delivery status only from merged evidence.

### Failure and recovery matrix

| Interruption or conflict | Required result |
| --- | --- |
| Proposal retained, worker disappears before report | Replay the same retained output and report identity; create one version without another paid call. |
| Provider response lost before retention | Preserve M2 uncertainty; do not invent a plan or automatically resend. |
| Proposal report duplicates or arrives after cancellation/fencing | Return the retained decision for an accepted duplicate; reject unauthorized new writes. |
| Version creation commits but response is lost | Exact retry returns the original version; no duplicate ordinal or content record. |
| Two revisions race | One expected-current update wins; the loser receives an actionable conflict. |
| Approval races a revision | One serialized result: approve the reviewed head or reject stale approval; never approve unseen content. |
| Crash during approval transaction | Receipt, all tasks and observation update commit together or none commit. |
| Approval commits before response or notification | Retry/reconnect recovers the same receipt and full task set from PostgreSQL. |
| Different approval keys race | One plan approval and one admitted task set; the loser receives an already-approved conflict. |
| Revision arrives after approval | Reject version creation; preserve the approved content and task set. A follow-up requires a new linked plan. |
| Broker unavailable after durable writes | Reads and receipts remain authoritative; any required outbox delivery stays pending. Broker storage cannot grant approval. |
| Slow reader, expired cursor or browser reload | Existing typed replay/expiry rules reconstruct plan and admission state without authorizing another command. |
| Scope spoofing or attempted parent deletion | Reject inconsistent ownership; preserve references needed by tasks and future feedback. |
| Retained state reopened after deployment restart | Project, conversation, version, approval and task identities remain unchanged and queryable. |

### Validation and evidence

Use the pinned toolchains and checks in [CONTRIBUTING.md](../../../CONTRIBUTING.md).
Runtime slices extend the existing PostgreSQL/JetStream and browser acceptance gates rather than creating a separate harness.
Keep deterministic fixtures separate from live-provider evidence; neither establishes production latency.

- Run formatting, Clippy, workspace tests/compile, frozen Node installation, TypeScript tests and generated-protocol checks.
- Run required Ayni quality/managed-lock, Helm, security and documentation checks for the affected scope.
- Deploy checkout-built images through `dev/ctl` using the explicit `docker-desktop` context.
- Interrupt proposal/report/acceptance and approval/observation boundaries; retain source revisions, workload identities and observed receipts.
- PR #238 includes runtime code for durable plan review and must satisfy the runtime checks above. The [plan review reference](../architecture/plan-review.md) records the concrete first-slice contracts.

## Whys

### Why carry feedback identity before implementing feedback?

M6 cannot reliably route a question if origin was inferred from mutable UI state.
M7 needs accepted task outcomes associated with the correct project and originating conversation.
M3 acceptance proves retained correlation without claiming that feedback consumption already exists.

### Why leave task execution to M4?

Execution needs repository verification, frozen source commits, Ayni runtimes and retained evidence.
Keeping this boundary makes approval safety independently reviewable.
Dependencies/concurrency, confirmed recovery and project assessment remain M5, M6 and M7 respectively.

## Where the detail lives

- [Plans and approval](../architecture/plans-and-approval.md): binding ownership, authorization, version and feedback invariants.
- [Roadmap](../../../ROADMAP.md): agreed delivery sequence and milestone boundaries.
- [Code organization](../architecture/code-organization.md): existing dependency and authority boundaries.
- [Conversation acceptance](../operations/conversation-acceptance.md): inherited failure cases and fixture/live evidence limits.
- [Local development](../operations/local-development.md): deployment topology, diagnostics and safe retained-state handling.
