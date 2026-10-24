# Operational acceptance

Run the cumulative operational profile in an owned Docker Desktop namespace:

```sh
AYNI_ORCHESTRATOR_NAMESPACE=ayni-orchestrator-acceptance-example \
  dev/ctl acceptance /absolute/new/evidence-directory --m8
```

The profile runs the existing M4 publication/failure matrix and M6 independent-task
recovery matrix, then adds six concurrent task admissions. With dispatch paused,
all six are durably queued. Once resumed, four distinct attempts receive distinct
Pod identities while two remain queued. A reviewed cyclic graph is rejected and
unselected dependency commits keep a dependent task blocked. A supervisor restart
must adopt the exact active identities. Cancellation then settles all six tasks;
control-plane, supervisor and PostgreSQL restarts must preserve terminal evidence
and exact start/cancel receipts.

The profile also runs deployed readiness and the ordinary conversation diagnostic.
A separate disposable PostgreSQL suite checks accepted multi-upstream selections,
retained recovery, bounded assessment consent and source-reclamation safety. These
are database integration results; the report preserves that boundary.

`acceptance.json` records completed and unmeasured gates and fingerprints retained
evidence. `outcome.json` reports whether the runnable profile passed. Neither a
passing profile nor its four-slot capacity observation declares all M8 acceptance
complete. Full closure still needs the separately identified deployed integration,
provider, retention-window, production-capacity and self-development evidence.
Live provider execution and live GitHub publication are absent from this fixture
profile.

Evidence and authoritative stores remain after the run. Use the existing local
cleanup workflow to stop the deployment while retaining data. Permanent namespace
or retained-store removal requires an explicit separate decision.

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
validates the bounded execution, managed-quality and draft-publication topology;
it does not establish autonomous coding capability. A passing host check, this
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

Set `AYNI_ORCHESTRATOR_SELF_DEVELOPMENT_RUNTIME_IMAGE` when running `--m8` to
include this profile in the cumulative evidence. Its passing topology evidence
does not establish general autonomous coding capability.
