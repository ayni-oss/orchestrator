# Task execution preparation

A trusted local operator prepares an admitted task for later coding execution.
PostgreSQL retains either explicit blockers or one immutable prepared attempt.
Preparation never creates a coding command, worktree, provider call or publication.
The [M4 delivery plan](../research/project-workflow-m4-delivery-plan.md) defines the
later worker, quality and publication boundaries.

The next [repository preparation extension](repository-preparation.md) will add
trusted server-side source verification and admitted runtime evidence with a
browser journey. It must retain this command's immutable scope, exact receipts,
approved inputs and separate start authority. Until that extension is delivered
and qualified, the local operator workflow below remains required.

## Authority and evidence

`PrepareTaskExecution` exists only on the trusted operator private-v1 listener.
It is deliberately absent from the browser mutation allowlist and model tools.
The operator attests source evidence; a digest supplied by an untrusted agent is
not verification. The `workflow-prepare-task` utility verifies trusted local Git
objects without network access, hooks, checkout filters or reading dirty source.
It compares origin to the approved repository, resolves the approved source ref or
explicit integrated source commit, and reads the exact `.ayni.lock` blob at that commit.
[Reviewed dependencies](task-dependencies.md) require explicit task/attempt/commit
selections; the helper verifies every selected upstream commit is an ancestor of
the integrated source. The source commit is the
frozen identity even if the ref later moves. Only SHA-1 Git repositories are
supported initially; lock blobs are bounded at 4 MiB. Local checkout access does
not establish remote fetch/push permissions.

The operator selects a digest-pinned runtime and numeric maximum time/token
budgets. Supported bounds are 1–3600 seconds and 1–200000 tokens. Runtime selection
does not establish image availability, platform compatibility, dependency readiness
or Ayni lock validity. Later workers must verify those using supported Ayni CLI
contracts before claiming execution readiness and accepted quality. Do not treat
`prepared` as worker authorization or bypass later checks.

A plan may review up to 64 exact `source_paths` for model context and coding
writes. Paths are relative, unique and bounded; traversal, symlinks and protected
inputs are rejected. This allows a small reviewed change in a repository whose
full source exceeds model context limits while preserving the full snapshot for
managed validation. An empty selection retains the existing full-tree bounds.

The complete approved task specification, including natural-language limits,
remains authoritative. Repository and source declarations cannot be substituted.
Unresolved approved inputs remain blockers; no free-text answer silently changes
approved scope. A linked follow-up plan is required to change the reviewed work. [Confirmed recovery](task-recovery.md)
can link another execution segment using the same frozen inputs and a newly approved allowance.

## Durable preparation

The command carries full workspace/project/conversation/task scope, idempotency
key and bounded inputs: exact repository/ref, resolved 40-character commit,
SHA-256 lock digest, digest-pinned runtime, execution budgets and exact upstream selections. Parent origin
comes from the retained task. The server freezes the current immutable workspace
agent configuration revision. All APIs stay v1.

Preparation serializes on the owning conversation. Exact receipt replay precedes
mutable eligibility checks. Changed input under the same key conflicts. A distinct
key after a prepared attempt returns `TASK_ALREADY_PREPARED`. At most 32 retained
preparations are allowed per task, including blocked receipts. Exhaustion rejects
new commands before acceptance; existing receipts remain replayable.

Missing evidence, runtime, configuration, budget or unresolved task inputs produce
a retained blocked receipt with no attempt ID. Complete input produces one
`prepared` attempt with constrained immutable workspace, project, conversation,
plan version/digest, approval and task references. Preparation result, attempt and
conversation observation update commit together or none commit. Runtime database
grants permit insertion and reading, never rewriting or deleting this evidence.

The receipt retains command/task/attempt identities, exact inputs, blockers,
configuration revision, timestamp and conversation revision/cursor.
`GetTaskExecution` reads the latest execution selection in full task scope. After recovery
it projects the linked attempt and its confirmation, while original preparation receipt
replay remains unchanged.
Older receipt replay does not replace the latest preparation. Admission state and
original approval receipts stay unchanged; prepared execution is a separate projection.
Watch updates use `TASK_EXECUTION_PREPARED`, with a safe reason distinguishing
blocked preparation from a prepared attempt. The task refresh view displays inputs,
blockers and frozen configuration.

## Operator workflow

Create a selection JSON containing full task scope and `inputs` with
`runtimeImage`, `maxSeconds` and `maxTokens`. The verifier fills repository, source,
commit, lock digest and an idempotency key. Use a trusted checkout containing the
approved Git objects; dirty working files are ignored.

```sh
dev/ctl prepare-task verify /absolute/trusted/checkout selection.json frozen-request.json
# Review the generated request before authorizing the selection.
dev/ctl prepare-task submit frozen-request.json
```

The utility never overwrites a frozen request. Preserve it through response loss
and resubmit the same file without re-resolving a moving ref or generating another
key. It contains no credentials. A blocked request may be submitted directly with
incomplete inputs to retain explicit blockers; resolving missing preparation inputs
uses a new key. The trusted operator boundary is a prerequisite, not a public API.

## Acceptance and handoff

The PostgreSQL gate covers blocked and ready inputs, exact/conflicting retries,
competing keys, cross-scope rejection, immutable origin/configuration, rollback,
reopened authority, and absence of turn/outbox/provider dispatch. The local Git
fixture proves dirty files cannot change the frozen lock digest. Deployed gates
exercise private-v1 preparation and receipt recovery through control-plane,
executor, broker and database restarts, including preparation during broker outage:

```sh
AYNI_ORCHESTRATOR_NAMESPACE=ayni-orchestrator-acceptance-m4-preparation dev/ctl deploy
AYNI_ORCHESTRATOR_NAMESPACE=ayni-orchestrator-acceptance-m4-preparation dev/ctl acceptance /absolute/new/evidence --tasks-only
```

The deployed preparation scenario uses synthetic source/image identities and
asserts no coding dispatch. It does not establish real image compatibility or
remote Git permissions; the separate local Git fixture verifies actual objects.

The [isolated execution contract](isolated-task-execution.md) defines PR 2. It must consume the prepared attempt without replacing its frozen inputs and
establish worker ownership/fencing before coding. PR 3 must verify Ayni runtime
compatibility and retain final evidence independently of agent-writable storage.
PR 4 owns publication authority and ambiguous external-action recovery.
