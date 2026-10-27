# Agent plan proposals

> UI status: the local product supports configuration and basic workspace/project
> navigation; see [integration connections](../architecture/integration-connections.md).
> Conversation/task browser steps below describe the retired UI and remain
> historical. Their acceptance profiles require replacement coverage. The API
> gateway itself serves no UI assets.

Conversation discussion can end in a structured proposal for review. The agent
can create a new plan in its originating conversation; it cannot approve a plan,
select another conversation, revise an existing plan, or dispatch coding work.
This extends [durable plan review](plan-review.md) and the
[plans and approval contract](plans-and-approval.md).

## Structured provider result

The existing foreground Responses request advertises `propose_plan` alongside
three scoped read tools. The agent discusses constraints and missing evidence in
ordinary conversation, then returns one terminal `propose_plan` function call.
It must be the sole call in that response. The worker does not invoke another
paid continuation after receiving a proposal.

The strict argument object has exactly three required fields:

| Field | Meaning and bound |
| --- | --- |
| `content` | Complete `PlanContent`, using the same snake-case fields, task bounds and 32,768-byte canonical limit as operator-authored versions. |
| `assumptions` | At most 16 nonblank strings, each at most 2,048 UTF-8 bytes. |
| `evidence` | At most 16 nonblank strings, each at most 2,048 UTF-8 bytes; these are declared references, not verified evidence. |

The complete proposal envelope is limited to 40,960 UTF-8 JSON bytes. Unknown
fields, NUL, malformed content, duplicate task keys and excess bounds are rejected
without truncation. No model-supplied parent identity is accepted. Open scope
questions remain visible in `content.unresolved_questions`; execution prerequisites
remain visible in each task's `unresolved_inputs`.

The provider loop checkpoints the normalized response before deriving its terminal
result. The result ledger retains the invocation identity and exact proposal JSON
before the supervisor publishes a signed report. The report signature binds the
proposal bytes, originating assignment/generation and provider-call evidence.
A retained response can reconstruct the same report without another provider call.
An unconfirmed sending outcome preserves the existing uncertainty rules.

## Application acceptance

The control plane derives scope from the authenticated assignment. It validates
the proposal against the admitted provider/configuration/context identity and
checks cancellation and current generation before creating a plan. Accepted
proposal content, immutable origin, observation update and terminal outcome commit
in one PostgreSQL transaction. Exact accepted report replay returns its retained
decision and cannot create another version.

The new version has author `agent`. Its optional `AgentPlanProvenance` records:

- originating turn, frozen configuration and context digest;
- retained provider call and response IDs;
- authenticated assignment and executor generation;
- signed report source, ID and digest, plus proposal invocation ID;
- provider-declared assumptions and evidence references.

Operator-authored versions have no agent provenance. Revising an agent version
creates a new operator-authored version and preserves the earlier version and its
origin. Read access continues to require the complete workspace/project/conversation
scope. Migration ordinal 13 retains constrained proposal/version/turn references;
referenced provenance cannot be rewritten through runtime permissions.

The proposal path preserves the existing per-conversation plan capacity and
immutable version retention. Malformed or excess proposals fail visibly; they do
not turn prose into tasks or grant additional provider calls. Cancellation and
fencing suppress new proposal writes. An already accepted report retains its
original decision even when replayed after cancellation or replacement.

## Console review and validation

The plan review panel shows assumptions, unresolved questions, declared evidence,
provider identity and authenticated producer provenance beside the exact content
and digest. Rendering uses text nodes. Reload and watch updates preserve accepted
versions and unsaved operator drafts. Repository/source references remain marked
as unverified declarations. Approval and task admission are later M3 work.

The cumulative provider fixture includes valid, malformed, scope-spoofing, mixed
read/proposal and delayed cancellation responses. The PostgreSQL and executor
gates cover retained replay and authority fencing. Run the deployed scenario with
`node apps/console/provider-acceptance.mjs` against the explicit Docker Desktop
stack; see [fixture setup](../../../dev/fixtures/provider/README.md).

For a separately reported live smoke, use the existing scoped credential mount
and `apps/console/provider-live-smoke.mjs` with `AYNI_SMOKE_PLAN_PROPOSAL=1` and
`AYNI_ALLOW_PAID_PROVIDER_SMOKE=1`. The script requires an idle isolated scope,
an explicit live model, at most 32,768 total tokens and 2,048 output tokens. It
admits one turn and checks retained proposal provenance and reconnect identity.
Failure or uncertainty requires inspecting that retained turn before another
paid attempt. Fixture results do not establish live-provider latency.
