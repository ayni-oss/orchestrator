# Conversation execution contract

This is the consolidated M2 entry contract for [configuration and isolated
sessions](../../../ROADMAP.md#m1m2-delivery-scopes). It carries forward the
[M1 handoff](../research/project-workflow-m1-durable-foundation.md#m2-handoff)
and resolves the configuration, session, provider and tool boundaries identified
by the [refactor research](../research/project-workflow-refactor-runbook.md).
The delivery PR owns implementation status and acceptance evidence. Provider
calls use the retained request lifecycle; tool execution and complete M2
acceptance remain PR 4.

## One v1 contract throughout alpha

All Orchestrator-owned interfaces remain **v1 until alpha merges**. Extend the
canonical `ayni.orchestrator.v1` API, `ayni.orchestrator.events.v1` payloads,
`build.ayni.*.v1` event names, `/v1` schema identifiers and retained format
contracts in place. Do not introduce parallel v2/v3 APIs, milestone-specific
schemas, compatibility adapters, or another research implementation. Keep
consumers, generated bindings, fixtures and documentation synchronized. The
existing private baseline remains `workflow-m1-private-v1`.

A contract's version is different from the identity of its data:

| Value | Meaning | Alpha rule |
| --- | --- | --- |
| Interface or format version | Interpretation of messages and persisted formats | Fixed at v1. |
| Configuration revision ID | One immutable snapshot of instructions, provider/model selection, budgets and session policy | New opaque ID when the configuration changes. |
| Conversation revision and event cursor | Ordering of accepted facts | Continue advancing. |
| Assignment and executor generation | Authority to execute one admitted turn | Continue advancing to fence old owners. |
| SQL migration ordinal | Ordered, checksummed installation step in one database schema | Continue forward without creating another interface. |
| Commit, image digest or dependency version | Identity of an external input or implementation | Keep exact pins; these are not Orchestrator interface versions. |

Existing unsupported-version rejection fixtures remain useful negative tests;
they do not establish supported v2 interfaces. Historical research identifiers
remain dated provenance. A change that cannot preserve admitted work requires an
explicit retained-state decision; the v1 rule never authorizes silently rewriting
receipts, replacing frozen inputs, or deleting retained stores.

## Shared configuration and frozen inputs

A workspace owns one current agent configuration and immutable configuration
revisions. All its projects and conversations share that configuration. One
revision identifies the complete snapshot: instructions, provider, model,
input/output/total token limits and session idle timeout. Instructions and
provider binding do not acquire independent interface versions.

A configuration update includes its expected current revision and a scoped
idempotency key. The transaction either installs one immutable revision and
advances the current pointer, or rejects the conflict. Replaying an accepted
command returns its original revision even after later edits. Configuration
contents never contain provider credentials.

Message admission freezes the current configuration revision atomically with its
receipt and turn. A later edit affects later admissions, including when an older
turn is still queued. Dispatch and FIFO promotion use the admitted revision;
provisioning must not reread today's instructions or model for yesterday's turn.
Turns admitted before configuration snapshots remain bound to their original
`scripted-v1` contract. They retain their original optional context tail without
retroactively acquiring configuration or a full context manifest.

The default provider/model is `scripted`/`scripted`. Selecting `openai` requires an explicit model identifier. Newly admitted OpenAI turns bind explicitly to the provider execution path.
Deployment enablement and a directly mounted scoped credential are also required
before network access. Previously admitted scripted turns retain `scripted-v1`.
Do not infer provider access or usage from configuration or scripted results.

At eligibility, freeze the originating conversation's bounded context through
its accepted predecessor outcomes and current user message. Later queued input,
other conversations' transcripts, plans and reasoning are excluded. Shared
configuration does not imply shared working memory. Immutable references and
digests preserve larger authorized inputs; exceeding a declared input limit is
an actionable failure, never silent truncation or an invented summary.

The context manifest retains at most 10,000 immutable message references. Admission
reserves capacity conservatively for retained messages, possible predecessor
responses and the new input. Exhaustion returns `context_manifest` capacity
failure before accepting the message. Successor promotion therefore cannot roll
back an accepted predecessor outcome because its context later exceeds the bound.

The event marks its inline tail complete or partial explicitly. `GetTurnContextInputs`
returns at most 100 records and a 64 KiB content/metadata budget per page, with a
stable ordinal cursor. These references survive watch cursor and history snapshot
expiry; those observation windows never rewrite admitted execution inputs.

## Isolated session authority

PostgreSQL owns session identity, desired state, assignment and generation.
Kubernetes provisions the workload; a Pod's existence cannot grant execution
authority. Each conversation's session has isolated context and retained ledger
storage. Session workloads receive only their assigned scope and credential
capabilities. They receive no control-plane database credential or authority to
accept their own reports.

| Boundary | Required behavior |
| --- | --- |
| Creation and wake-up | Retain desired session identity/generation before provisioning; retry converges on the same workload. A queued turn waits visibly for preparation. |
| First claim | Accept only the application-issued assignment and current generation. A racing executor cannot acquire another conversation or replace the winner. |
| Restart adoption | Reuse the retained session identity, ledger and exact pending reports after verifying scope and generation. An empty replacement ledger is not adoption. |
| Idle expiry | Stop only idle sessions with no unresolved provider request or report handoff. Retain durable recovery evidence and wake on later eligible work. |
| Replacement | List the advisory `can_replace` flag after turns and outbox handoffs settle. The replacement command rechecks authority transactionally before fencing the generation; later overlap cannot admit old-generation progress or terminal results. |
| Cleanup | Release workload resources independently of accepted conversation history. Do not remove unresolved command, request, report or decision evidence. |

Fencing prevents stale reports from changing accepted state; it cannot revoke an
external call already sent. Before replacing a session that might have sent a
provider request, use the retained request state below. Never turn workload
replacement into an implicit retry of a possibly paid call.

## Provider requests and cancellation

OpenAI is the first provider. Execution uses an explicit model, foreground Responses
requests and client-managed conversation context. The [provider request contract](provider-requests.md)
defines request intent, budget reservation, uncertainty, retained results and
cancellation. Its state machine and recovery matrix must pass before paid calls
are enabled. Configuration selects the admitted binding; deployment policy and scoped credentials
gate its execution.

## Bounded tool authority for PR 4

The initial tool set consists only of scoped reads:

| Tool | Allowed input and output |
| --- | --- |
| `read_workspace_instructions` | Read the instructions in the turn's frozen configuration revision. |
| `read_project_status` | Read the originating project's objective and factual status at an explicitly retained snapshot. No other conversation transcript, plan or reasoning. |
| `read_conversation_history` | Read bounded accepted history from the originating conversation through the frozen eligibility boundary. |

Each invocation carries the turn/assignment/generation, a stable invocation ID,
validated arguments, an input digest and bounded retained output. Tools cannot
expand scope from model-provided IDs. Initial limits are at most eight tool
invocations per turn, 4 KiB argument bodies and 32 KiB output per invocation;
all content counts toward the admitted input allowance. Reject over-limit reads
explicitly. Repeated invocation IDs replay the same retained read result.

These tools have no shell, filesystem mutation, repository checkout, arbitrary
HTTP, provider-native web/code tools, MCP, approval, publication or merge
authority. Instructions and model text cannot grant a capability. Coding
worktrees and Ayni task execution remain M4; MCP remains M7.

### Activity and credential projection

Activity projects only allowlisted stages, identifiers, bounded counters and
safe reason codes: provisioning, agent wake-up, preparation, provider request,
uncertain outcome and tool execution. Never project API keys, full prompts,
model reasoning, tool arguments, raw provider errors or unrestricted output as
activity detail. Transcript content remains subject to its own conversation
scope and retention rules.

Inject provider credentials directly into the
isolated session using scoped Kubernetes secret mechanisms; keep them out of
gRPC, JetStream, context manifests, logs and retained transcripts.

## Cumulative acceptance

| Delivery scope | Required evidence |
| --- | --- |
| PR 2 | Configuration edit/replay/conflicts, admission freezing, two-conversation isolation, wake-up, expiry, adoption, replacement and stale-generation rejection. |
| PR 3 | Prepared/sending/result boundaries, uncertainty, cancellation, retained usage and exactly-once visible acceptance. |
| PR 4 | Bounded tools, cleanup/reconnect matrix, repeatable provider fixtures and a bounded live-provider smoke test. |

Exercise affected runtime behavior on the explicit Docker Desktop Kubernetes
context and report blocked checks. Scripted fixtures cannot establish real
provider recovery or latency.
