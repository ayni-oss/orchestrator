# Conversation acceptance

> UI status: the console and initial product workspace are removed. Browser steps
> and evidence below describe the retired UI and are historical reference only.
> Browser-dependent acceptance profiles are unavailable until replacement coverage
> is implemented. The current local API adapter is `dev/ctl gateway`; it serves
> no UI. See [the retained UI example](../../../examples/web/EXAMPLE.md).

Run these gates for the final M2 slice. The delivery PR records actual results,
source revision, deployment identity and blocked checks. This matrix defines the
required evidence; listing a command does not mean it passed.

## Cumulative matrix

| Boundary | Required observation | Executable gate |
| --- | --- | --- |
| Configuration and eligibility | Admissions keep their immutable revision; eligible context and project status stay scoped and frozen. | PostgreSQL configuration/provider tests in `scripts/ci/check-workflow-m1.sh`. |
| Session lifecycle | Two conversations use separate PVCs and generations; restart adopts retained state; replacement fences stale owners. | PostgreSQL session tests, worker tests, `pnpm console:acceptance`, `scripts/ci/check-session-topology.py`. |
| Preparation | Missing credentials or input allowance fail before sending; later provisioning can load the scoped mount. | Worker provider-session tests. |
| Provider sending | Crash or lost response remains uncertain and does not send twice. | Worker sending/restart tests and deterministic provider disconnect mode. |
| Retained result | Restart republishes one result; the application accepts one response and keeps nullable usage factual. | Worker result replay, PostgreSQL acceptance tests, browser provider acceptance. |
| Scoped tools | Three reads use frozen inputs; unknown tools, model scope arguments, excess invocations and oversized outputs fail explicitly. | Provider/worker tool tests and fixture browser acceptance. |
| Budget and cancellation | Every continuation counts toward the admitted budget; cancellation stops future work and preserves possible charges. | Worker budget/cancellation tests, PostgreSQL cancellation race tests and provider browser acceptance. |
| Explicit uncertainty recovery | Exact-call acknowledgement is idempotent, preserves unknown usage and retires the held generation before FIFO promotion. | PostgreSQL uncertainty tests and provider browser acceptance. |
| Observation | Refresh, two tabs, reconnect, expired cursors and slow readers retain scope and accepted work. | `pnpm console:acceptance`, provider browser acceptance and PostgreSQL watch tests. |
| Cleanup | A matching application decision precedes compaction; unresolved provider, tool and report evidence survives. | Worker ledger tests, integrated durable handoff tests and topology checks. |
| Real provider | One explicitly enabled bounded turn uses the actual provider, tools and scoped credential mount. | `apps/console/provider-live-smoke.mjs`, separately reported from fixtures. |

## Local commands

Use the pinned toolchains and explicit Docker Desktop context from
[local development](local-development.md). The main cumulative gate creates its
own disposable PostgreSQL and authenticated JetStream fixtures:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo check --workspace --all-targets
scripts/ci/check-workflow-m1.sh
pnpm typecheck
pnpm test
pnpm proto:check
pnpm chart:check
pnpm docs:check
```

Deploy the checkout and run `dev/ctl check`, `dev/ctl scenario` and browser
acceptance against that deployment. The
[provider fixture guide](../../../dev/fixtures/provider/README.md) defines its
local provider endpoint and the separate live smoke prerequisites.

## Evidence limits

Deterministic fixtures establish request mapping, authority, replay and failure
handling. Their timings do not predict real-provider latency or availability.
The initial tool profile rejects responses combining reasoning and function
calls, because it does not retain reasoning for subsequent requests.
A live smoke requires an explicitly enabled deployment, an isolated workspace
with a directly mounted provider credential and a bounded explicit model.

Keep unresolved ledgers and PVCs. Acknowledging possible charge is a recorded
operator decision, not proof of a refund, absent generation or known token usage.
NetworkPolicy manifests express policy; a topology check does not establish that
the cluster network plugin enforces it.
