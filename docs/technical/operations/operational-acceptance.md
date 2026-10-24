# Operational acceptance

Run the cumulative local profile against a committed checkout in an owned Docker Desktop namespace.
Build and load the digest-pinned managed runtime for the full Orchestrator source before starting.
The profile requires the task-quality fixture runtime, local Rust/Node tooling and installed Playwright Chromium.
It rejects a missing own-source runtime before running the expensive deployed gates.

```sh
AYNI_ORCHESTRATOR_NAMESPACE=ayni-orchestrator-acceptance-example \
AYNI_ORCHESTRATOR_SELF_DEVELOPMENT_RUNTIME_IMAGE=docker.io/local/orchestrator-runtime@sha256:REPLACE_WITH_ACTUAL_DIGEST \
  dev/ctl acceptance /absolute/new/evidence-directory --m8
```

## Required local evidence

| Gate | Observable result |
| --- | --- |
| Cumulative M4 and M6 | Publication, cancellation, pod loss, fenced reports and retained cleanup replay their accepted identities. |
| Dependencies | Two accepted upstream draft commits become explicit, frozen inputs to a dependent attempt and its confirmed repair. |
| Worker recovery | A worker question and model proposal require an answer and exact confirmation before bounded continuation. |
| Outcome assessment | Accepted worker results trigger bounded assessment; changed outcomes reject stale proposals and consent survives response loss. |
| MCP and providers | OpenAI and Anthropic protocol fixtures exercise tool consent, external-effect uncertainty, cancellation and retained replay after pod replacement. |
| Own-source execution | Reviewed Orchestrator source passes actual managed quality, retains one draft, and preserves cancellation and replay across owner restarts. |
| Retention and cutover | Disposable PostgreSQL tests exercise persisted grace-period expiry; the exact private-v1 inventory rejects obsolete entry points. |
| Diagnostics and capacity | Readiness, lifecycle/storage metrics, hardware, resource limits and image/source/lock identities accompany four active and two queued attempts. |

The capacity scenario first pauses dispatch so all six admissions remain durably queued.
Resuming dispatch must produce four distinct active Pod identities and two queued attempts.
Supervisor restart preserves those identities; cancellation and owner restarts preserve exact start/cancel receipts.
The cycle and unresolved-upstream checks reject work before execution.

## Reading the report

`acceptance.json` validates required profile schemas, gates, retained artifact fingerprints and frozen source identity.
Incomplete profiles, changed artifacts, duplicate attempts and changed adoption identities fail aggregation.
`outcome.json` reports failure when any required profile fails, including own-source execution.
Both reports retain the distinction between local completion and milestone qualification.

| Field | Meaning |
| --- | --- |
| `cumulative_profile_complete` | Every required runnable local profile passed. |
| `local_acceptance_complete` | Required deployed local scenarios and managed self-development passed. |
| `milestone_complete` | Remains false while live additional-provider qualification and sustained production capacity remain unmeasured. |
| `artifacts_sha256` | Fingerprints of retained inputs, reports and observations for review. |

The provider and coding profiles use explicitly deterministic fixtures; publication uses real Git transport and a local GitHub protocol fixture.
These results establish protocol and recovery behavior without asserting autonomous coding ability, live-provider behavior or production throughput.
The accelerated persisted-clock retention tests establish grace-period policy correctness without waiting thirty days.
They do not measure production storage growth.

Evidence and authoritative stores remain after the run, including the MCP session namespace and ledger.
Use the existing local cleanup workflow to stop deployments while retaining data.
Permanent namespace or retained-store removal requires an explicit separate decision.

## Retained source archives

The minute maintenance sweep starts a 30-day grace period when a task run becomes
eligible for archive retirement. The grace period begins when eligibility is first
observed, rather than at task completion. Each sweep marks at most 32 archives,
reclaims at most 32 expired archives, and resets at most 32 grace periods whose
eligibility changed. Restarting a process preserves these database timestamps.

Retirement requires a terminal coding run with no sending or uncertain provider
call, plus either a retained draft publication or an explicitly confirmed child
attempt holding its own source archive. A publication still awaiting resolution,
an unanswered recovery question, or a recovery proposal without a confirmed
parent decision protects the source. Any parent confirmation resolves competing
proposals under the existing once-only continuation authority.
The sweep rechecks eligibility while holding the task and publication authority
locks. Losing eligibility resets the grace period; later eligibility starts a new
30-day window. Active work and unresolved external outcomes remain protected.

Only `workflow_task_runs.source_pack` is cleared. The immutable command and its
source digest, accepted patches, managed-quality artifacts, plan approvals,
publication identities, recovery records, and exact command/report receipts remain
retained. The database stores retirement time and the reclaimed byte count.
A retired source cannot authorize fresh execution or recovery; a source-dependent
continuation fails with an execution precondition conflict. The system does not
reconstruct archived inputs from a mutable remote branch.
This policy does not delete session volumes, erase transcripts, purge broker or
executor authority, or reset a namespace. Those stores retain their existing
bounded-cleanup and explicit disposal workflows.

## Diagnostics and capacity

`/readyz` checks schema history, the catalog manifest and exact runtime privileges.
`/metrics` exposes task counts and retained source bytes by lifecycle state,
publication counts by state, retired archive counts, reclaimed bytes, and sources
in their grace period. Existing conversation, outbox and delivery backlog metrics
remain available. These projections contain identities and counts rather than
transcripts, patches, credentials or raw provider responses; they never change
workflow authority.

The four-active/two-queued test establishes the local four-slot dispatch bound and
restart adoption under the fixture workload. It does not measure sustained
throughput, realistic provider latency, storage growth under real artifacts, or
production resource sizing. Record hardware, resource limits, source/image
identities and workload shape alongside any later capacity evidence.

## Cutover and self-development

The replacement is a clean-slate workflow. There is no migration or dual-write
path from the superseded SQLite workflow. The private-v1 feasibility gate checks
the exact maintained RPC inventory and rejects obsolete/custom-client workflow
entry points in current application and deployment paths. Retained research under
`docs/technical/history/` describes provenance and is not executable compatibility
code. Upgrades of this replacement still apply its own forward PostgreSQL
migrations and preserve accepted authority.

A self-development gate must start from an explicitly reviewed plan against a
frozen Orchestrator source commit, execute in isolated worktrees, validate with
the actual managed Ayni runtime, retain complete quality evidence and end at a
reviewable draft. Capture cancellation and restart behavior and the exact
source/image/lock identities. The own-source deterministic coding profile below
validates execution, managed quality, cancellation, restart replay and draft publication.
It does not establish autonomous coding capability. A passing host check, this
implementation PR, or manual commits alone do not satisfy the deployed gate.
Users retain merge and deployment authority. Keep `milestone_complete` false
until every recorded prerequisite and cumulative deployed gate has passing
execution evidence.

## Deployed outcome assessment scenario

With the contributor console and deterministic provider fixture forwarded locally,
run this separate profile against an owned acceptance namespace with scripted task
execution and the managed quality runtime enabled:

```sh
AYNI_ORCHESTRATOR_NAMESPACE=ayni-orchestrator-acceptance-example \
  CONSOLE_URL=http://127.0.0.1:3000 \
  PROVIDER_FIXTURE_URL=http://127.0.0.1:18080 \
  AYNI_TRUSTED_GRPC_ADDRESS=127.0.0.1:50051 \
  node apps/console/assessment-acceptance.mjs /absolute/new/assessment-evidence
```

The script creates its own workspace, two approved tasks and a public fixture-only
provider Secret. Source preparation and upload use the trusted `workflow-prepare-task`
CLI over the explicit loopback gRPC forward; the browser gateway remains read-only
for those source operations. The task commands freeze scripted execution before the workspace
switches to fixture conversation execution. Dispatch is paused only in the owned test namespace so a durable queued task spans
a maintenance sweep; that progress must admit no model call. Dispatch then resumes
and the actual worker reports its result.
Accepted coding then triggers a bounded assessment. A second worker's accepted
result changes the snapshot while the first fixture response is delayed, proving
stale response/proposal rejection. Subsequent current outcomes produce a retained
agent plan for review, without adding tasks. The console consent command loses
its response and replays the same scope, allowance, configuration and key after
reload. Revocation and exact proposal identities survive conversation-supervisor
restart without repeating provider calls.

`outcome.json` records the current stage even on failure. Passing evidence includes
the real managed-quality results, retained call/proposal identities and a console
screenshot. Coding and provider behavior remain explicitly deterministic fixtures;
this profile establishes no live-provider capability or project completion.

## Own-source self-development fixture

Supply the digest-pinned managed runtime for this repository, rather than the
small task-quality fixture runtime:

```sh
dev/ctl self-development /absolute/new/self-development-evidence \
  docker.io/local/orchestrator-runtime@sha256:REPLACE_WITH_ACTUAL_DIGEST
```

This profile freezes the committed Orchestrator source tree, reviews an exact
`README.md` read/write selection, executes one explicitly deterministic coding
fixture, runs actual managed Ayni quality against the full source, and retains a
draft through the local GitHub protocol fixture and real Git transport. It uses
a disposable trusted checkout with the fixture repository identity; its report
separately records the actual Orchestrator source commit, full source-pack digest,
lock, runtime, quality and resulting draft commit. It never publishes to live
GitHub or invokes a paid coding provider. The source, evidence and fixture remote
remain available for inspection.

Read-only attempt polling retries transient transport cancellation, unavailability
and deadline errors up to three times with bounded backoff. Each retry is retained
in `read-only-poll-retries.jsonl`; terminal task and quality failures still fail
the profile immediately.

`--m8` requires `AYNI_ORCHESTRATOR_SELF_DEVELOPMENT_RUNTIME_IMAGE` and includes this profile in its cumulative evidence.
A second reviewed task against the same frozen source is cancelled while queued.
Supervisor, publisher and PostgreSQL restarts must preserve the successful draft, cancelled attempt and exact receipts.
