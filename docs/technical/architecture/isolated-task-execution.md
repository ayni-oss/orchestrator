# Isolated task execution

An operator starts an exact prepared attempt with a bounded Git snapshot pack.
One task runs at a time, in its own checkout and retained PVC. Accepted changes
remain a patch and workspace for review. The separate [quality boundary](task-quality.md)
validates a frozen reconstruction and retains Ayni evidence. Git push, publication,
automatic repair and cross-task scheduling belong to later slices.

## Durable dispatch and authority

`StartTaskExecution` binds full originating scope, prepared attempt ID, exact
source-pack bytes and an idempotency key. It retains the immutable command, pack,
queued state, command receipt and observation in one PostgreSQL transaction.
The command copies the prepared inputs and their configuration revision; later
configuration edits and moving repository refs do not affect it. A second start
key cannot create another run for the attempt. Exact retries recover the original
receipt after later state changes.

Task dispatch uses a bounded PostgreSQL queue and the existing authenticated
supervisor authority pattern, rather than adapting conversation-turn broker
messages into coding commands. JetStream continues to carry conversation handoffs;
its availability neither authorizes nor blocks a task start. The trusted task
supervisor polls `/v1/task-supervisor` over authenticated HTTP. A lost response
returns the same retained active command on the next poll. This is a durable
application handoff, without a second broker ownership decision or a new stream.

A global transaction lock and unique active-slot index allow one claimed,
reporting or cancelling task. The queue holds at most 32 pending tasks. Claiming
freezes a deadline from the prepared time limit. Claims survive process restart;
reconciliation reuses the same Pod identity, command and source bytes.

## Workload isolation

The supervisor provisions only scoped Pods, PVCs and capability Secrets in the
existing dedicated worker namespace. Workers have no database or broker identity,
Kubernetes token, publication credential or listener. A worker-specific HMAC bearer
binds one attempt and original generation; it cannot authorize supervisor commands
or another task. Every request also carries the Pod UID retained by PostgreSQL.
The supervisor verifies workload labels, command digest and UID before adoption or
delete, and uses Kubernetes UID/resource-version deletion preconditions.

A trusted initializer installs the worker executable in a read-only runtime volume
and reconstructs the exact source commit from Git objects. The pack includes the
commit and snapshot trees/blobs, not repository history, credentials, a working
index or mutable local files. Git hooks, filesystem monitors and external filters
are not enabled. The original lock blob must match the prepared digest. Submodules
are unsupported and fail explicitly. Packs are bounded at 8 MiB.

The coding container uses the operator's digest-pinned runtime image. It invokes
the trusted worker directly for bounded file proposals. Dependency preparation
and final quality commands run through the separate credential-free
[validation boundary](task-quality.md). Ayni's image entrypoint verifies the
attached lock and protected runtime content there; `env doctor` is an engine-host
image lifecycle diagnostic and is not used as in-container validation. The worker never
uses a checkout-supplied command or shell tool.

## Bounded coding and paid-call recovery

The initial coding capability is one OpenAI Responses file-proposal pass. The model
receives frozen instructions, the reviewed task specification and a complete bounded
text snapshot: at most 64 files and 128 KiB. Exceeding those limits fails explicitly;
no context is silently truncated. It can propose full UTF-8 contents for up to 16
files and 32 KiB of changed content. Deletion, shell execution, symlink traversal,
`.git`, `.ayni`, `.ayni.lock` and `.ayni.toml` writes are unavailable. File operations
use directory descriptors and no-follow opens. The resulting patch is bounded at
60 KiB and retained outside the tool-accessible checkout.

The configured provider must be enabled explicitly and have its workspace-scoped
credential mounted directly into the coding container. The initializer and supervisor
receive no provider credential. Network policy permits only the task authority,
DNS and, when enabled, the configured public HTTPS provider capability or explicitly
selected local fixture. Source code is never executed with these credentials.

Before sending, the worker durably records exact request bytes in its PVC ledger,
then consumes one application-owned sending permit. A consumed permit cannot
authorize another call. The response is retained before any file proposal is applied;
a retained response or result can be replayed without another paid request.
A crash in the sending window, lost reservation response or failed retention
produces uncertainty. A missing ledger cannot obtain a second permit. Usage remains
nullable when unknown. Calls obey the prepared token budget and retained deadline.

The worker retains its final outcome before reporting it. The application accepts
one bounded result under the current generation and registered Pod UID. Identical
report retries recover acceptance; conflicting bytes are rejected. A reporting
attempt still occupies the serial slot until the supervisor confirms the Pod is
terminal. Source, ledger and accepted outcome are retained across restarts.

## Cancellation, loss and cleanup

`CancelTaskExecution` retains an exact receipt and advances the fence in the same
transaction as the cancellation observation. Queued work becomes cancelled without
a Pod. Claimed/reporting work becomes cancelling; old-generation worker input,
paid-call admission and new reports are rejected immediately. An already sent paid
call may have an uncertain charge; cancellation never authorizes resending it.

The slot is released only after the trusted supervisor confirms the owned Pod has
disappeared, or accepts its terminal state. Pod loss after registration becomes an
interrupted run with retained work and possible provider uncertainty. It does not
silently create a replacement Pod or retry coding. Confirmed recovery belongs to
M6. A process/container restart inside the same Pod can use the original ledger;
a retained sending record still prevents another paid call.

Completed/failed Pods, source PVCs and scoped capability Secrets remain inspectable.
The local cleanup command stops owned workers while preserving all authoritative
volumes. Retention and reclamation policies belong to operational acceptance.

## Operator flow and observation

After verifying and submitting preparation, freeze the exact start request:

```sh
dev/ctl prepare-task start /absolute/trusted/checkout frozen-request.json frozen-start.json
dev/ctl prepare-task submit-start frozen-start.json
```

Retain the start file for response-loss retries. It contains a source snapshot,
so store it under the same access boundary as the checkout. Starting work is a
trusted local command; browser and model tools cannot upload or authorize source.
The console can read attempt state, retained patch, frozen identities and provider
uncertainty, and can explicitly cancel an attempt. Its **Tasks** panel refreshes the selected
attempt, separates coding from quality and publication, and provides reviewed
publication controls under the [console task contract](contributor-console.md#task-observation-and-publication-controls). `GetTaskAttempt` enforces full
origin scope. Task changes emit `TASK_EXECUTION_CHANGED` observation updates.

`taskExecution.fixtureEnabled` is disabled by default. Its scripted worker writes
a deterministic fixture file and bypasses Ayni/provider execution. It tests real
Kubernetes ownership, source reconstruction, retained changes and cancellation;
it does not establish live provider or Ayni runtime compatibility.
