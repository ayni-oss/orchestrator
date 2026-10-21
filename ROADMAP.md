# Roadmap

This roadmap records agreed product direction. Requirements, acceptance criteria, evidence, and delivery status belong in the issue and pull request for each bounded slice. Development branches start from current `origin/alpha` and target `alpha`.

## Project workflow replacement

The replacement is clean-slate. PostgreSQL is authoritative transactional state, JetStream will carry durable delivery, and gRPC provides command and observation contracts. Prior SQLite workflow schemas, retained data, APIs, tests, browser paths, and broker topology are research input only: do not migrate, dual-write, preserve, or adapt them.

The [M1 durable foundation contract](docs/technical/research/project-workflow-m1-durable-foundation.md) is binding for M1. The [refactor runbook](docs/technical/research/project-workflow-refactor-runbook.md), [worksheets](docs/technical/research/project-workflow-design-worksheets.md), and [feasibility evidence](docs/technical/research/project-workflow-m1-feasibility-evidence.md) preserve the investigation and approved boundaries.

### Product constraints

- Conversations share project configuration but retain isolated working context.
- Plans originate in conversations and require explicit approval of an immutable version.
- Concurrent tasks use isolated worktrees and freeze selected source commits.
- Questions and failures return to their originating conversation.
- Task delivery ends at a reviewable draft; users retain merge authority.
- Workspace instructions and MCP integrations precede multiple providers.
- Tenancy expansion and user management remain outside this replacement.

### Delivery sequence

| Milestone | Observable outcome |
| --- | --- |
| M1: Durable foundation | PostgreSQL persistence, durable delivery, one scripted executor, canonical private-v1 commands, snapshots, history, and resumable observation survive failures. |
| M2: Conversation execution | Shared agent configuration, isolated sessions, workspace instructions, one provider, and structured activity. |
| M3: Plans and approval | Conversation-owned immutable plan versions and exactly-once approval receipts admit the reviewed task set. |
| M4: Isolated task execution | Ayni runtimes, worker supervision, retained evidence, and draft publication expose separate execution, quality, and publication outcomes. |
| M5: Dependencies and concurrency | Concurrent worktrees and explicit upstream commit selection keep dependent inputs frozen. |
| M6: Feedback and recovery | Attention items, retained work, recovery proposals, and confirmation survive pod loss. |
| M7: Project continuity and integrations | Factual summaries, outcome assessment, proactive proposals, MCP integrations, and additional providers respect configured scope. |
| M8: Operational acceptance | Retention, diagnostics, capacity, self-development, cutover, and obsolete-path removal pass cumulative failure tests. |

### M1 runtime slices

M1 is delivered as cumulative, executable slices rather than one unsafe cutover.

1. **PostgreSQL authority and canonical private-v1.** Minimal hierarchy, scoped idempotency, FIFO message admission, exact outbox bytes, snapshots/history, readiness, trusted-loopback APIs, local PostgreSQL, and deletion of superseded callers.
2. **Durable scripted handoff.** Isolated authenticated JetStream streams, a retained scripted executor, signed ownership claims, application report decisions, accepted responses, and FIFO successor promotion survive application, broker, and executor restarts without treating broker storage as ownership.
3. **Resumable observation and cumulative acceptance.** Notification-backed watch replay, quarantine, retention cleanup, metrics, version-skew/capacity evidence, and interruption at every commit/publication/claim/report/decision/ack boundary prove no accepted work or retry bytes are lost.

Slice 3 implements `WatchConversation` using an atomic PostgreSQL snapshot boundary and retained update replay. The cumulative acceptance harness exercises admission, broker delivery, ownership, reports, decisions, replay, capacity, and retained-state recovery. M1 does not deploy providers, browser UX, or custom-client workflow access. Later milestones build on the PostgreSQL and retained handoff authorities.

### Validation at every milestone

| Level | Evidence |
| --- | --- |
| Contract | Valid transitions, exact API surface, scoped identities, and frozen inputs. |
| Integration | Real storage/transport, migrations, retries, and projections. |
| Failure | Controlled interruption at every durability boundary. |
| Operator scenario | Observable command, progress, reconnect, recovery, and cleanup behavior. |

Keep one cumulative acceptance harness. A passing feasibility experiment does not make its mechanism production runtime behavior. Each delivery PR names prerequisites, observable outcomes, failure cases, validation commands, and obsolete paths to delete.

Ayni's independent CLI roadmap belongs in its own repository.
