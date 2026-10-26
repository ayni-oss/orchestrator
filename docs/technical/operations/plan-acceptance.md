# Plans and approval acceptance

> UI status: the console and initial product workspace are removed. Browser steps
> and evidence below describe the retired UI and are historical reference only.
> Browser-dependent acceptance profiles are unavailable until replacement coverage
> is implemented. The current local API adapter is `dev/ctl gateway`; it serves
> no UI. See [the retained UI example](../../../examples/web/EXAMPLE.md).

M3 extends the [conversation acceptance](conversation-acceptance.md) gates.
The final delivery PR records executed results, source and image identities,
fixture/live distinctions and failed or blocked checks. This matrix is a gate
specification; it does not by itself establish a passing deployment.

## Cumulative matrix

| Boundary | Required observation | Executable gate |
| --- | --- | --- |
| Immutable review | Lost create/revise responses replay exact versions and receipts; competing revisions cannot replace unseen content. | PostgreSQL `plans`, API/gateway and `pnpm console:acceptance`. |
| Proposal retention | Retained output/report replay creates one version without a second provider request; cancellation/fencing and malformed, oversized, mixed or foreign-scope proposals cannot write. | Worker proposal tests, PostgreSQL `plan_proposals`, durable handoff tests and provider fixture acceptance. |
| Operator authority | A proposal alone admits no task. Resolve scope, review an exact version and explicitly consent; admission retains its original proposal history/provenance. | `node apps/console/provider-acceptance.mjs`. |
| Approval races | Same-key replay recovers one receipt; different keys and concurrent revision serialize into one approved head and complete task set. | PostgreSQL `plan_approval` and browser two-tab approval. |
| Atomic admission | Failure after receipt/first-task insertion rolls back tasks, receipt, revision and observation together; retry then admits the full set. | PostgreSQL `plan_approval` trigger interruption. |
| Scope and retention | Wrong digest/scope, changed retry content, missing acceptance and unresolved scope fail. Parent deletion and origin rewrites fail; runtime grants preserve retained references. | PostgreSQL plan/proposal/approval and API/gateway tests. |
| Observation | Reconnect/reload, expired cursors and slow readers reconstruct accepted state. Stored plan updates replay with identical bytes and revision order after restarts. | Browser acceptance, PostgreSQL watch tests and `dev/ctl acceptance`. |
| Broker outage | With JetStream stopped, plan create/revise/approve and reads work. Control-plane restart recovers exact receipts, versions, tasks and watch updates before the broker returns. | `dev/ctl acceptance`, `plans-broker-unavailable.json`. |
| Retained restart | Control plane, supervisor, JetStream and PostgreSQL restart individually without changing versions, approval/task identities, specifications, blockers or origin. | `dev/ctl acceptance`, `plans-replay-*.json`. |
| Execution handoff | Admission leaves tasks awaiting execution and creates no turn/dispatch/outbox work or provider request. | PostgreSQL `plan_approval`, plan client snapshots and provider fixture counters. |

## Run and inspect

Run the checks in [CONTRIBUTING.md](../../../CONTRIBUTING.md), including
`scripts/ci/check-workflow-m1.sh`. Its real PostgreSQL/JetStream fixtures include
all three M3 storage suites and invoke the deployed plan acceptance client against
a restarted gRPC listener with no broker/executor present.

Use the explicit Docker Desktop topology in [local development](local-development.md).
Choose an isolated namespace; preserve all retained data:

```sh
export AYNI_ORCHESTRATOR_NAMESPACE=ayni-orchestrator-acceptance-m3
dev/ctl deploy
dev/ctl check
dev/ctl scenario
dev/ctl acceptance /absolute/path/to/new-evidence-directory --plans-only
dev/ctl console
pnpm console:acceptance
```

The `--plans-only` profile runs M3 broker-outage and retained-restart correctness
without claiming latency results. Omitting it runs the full inherited M1 profile
as well; failed performance gates must be reported separately from M3 correctness.
Both profiles compare original PVC names, UIDs and bound volume identities.

The full client warms one conversation/session before taking 100 scripted samples;
fresh-session provisioning is excluded from these warmed latency gates.
Exact receipt/watch replay and aggregate message/turn counts are checked after
every sample and after retained restarts.

The existing acceptance command saves source revision/diff/manifest, workload
image IDs, original protobuf response bytes and original watch updates. During
the broker outage it creates and revises a two-task plan, approves the reviewed
version, verifies exact retries, and repeats verification after each restart.
Evidence directories must be new. A failure records `outcome.json` and restores
a temporarily scaled-down dependency; a failed run is never passing evidence.

The provider fixture runs separately using its
[existing guide](../../../dev/fixtures/provider/README.md). In addition to the
proposal failure cases it follows an agent proposal through scope resolution,
operator review/consent, admission and reload. It checks original task content,
origin/provenance and no additional provider call. Deterministic fixtures do not
establish live-provider latency. PR #239 records the separately executed bounded
live proposal smoke; repeat it only with explicit paid-call authorization.

## Operations and downstream boundary

Recover an ambiguous command using its original scope, key and content. Query
`GetPlanApproval`, `GetPlanVersion` and `GetTask` under the full originating
conversation scope. Resume observation from its cursor, or take a fresh snapshot
after typed expiry. Broker delivery cannot authorize approval. Never repair
uncertainty by replacing keys or deleting receipts, ledgers or PVCs.

M4 must verify repository/source inputs, freeze the selected commit, select an
Ayni runtime and authorize execution limits before dispatch. Preserve workspace,
project, originating conversation, plan version/digest, approval and task IDs in
attempts and reports. M6 returns questions/failures to that origin; M7 uses accepted
outcomes for factual project continuity. Admission evidence proves none of those
later execution or feedback capabilities.
