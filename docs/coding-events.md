# Coding lifecycle reporting

New physical coding executions select `coding-report-v1` when the configured
runtime reporting mode is `events`. The database records the selection before
Job creation. Existing attempts migrate with their legacy reader, including
publication-only recovery; changing the deployment mode does not reinterpret
admitted work. Schema 17 adds `coding_executions` and `coding_inbox`.

## Authority and retained evidence

`CodingReportAvailable` uses generated Protobuf/ProtoJSON inside structured
CloudEvents. New types use Ayni's canonical `ayni.build` domain:
`ayni.build.orchestrator.coding.report.available.v1` and
`ayni.build.orchestrator.coding.accepted.v1`. Its binding identifies task, attempt, physical generation, Job,
Pod UID, frozen snapshot digest and exact worker-request digest. The trusted
supervisor retains immutable JSON under
`coding-events/<generation>/<pod-uid>/<event-id>.json` before sending the event.
The envelope references its SHA-256 and size. Reports are bounded to 512 KiB;
envelopes are bounded to 16 KiB. Paths are derived from validated identifiers,
never supplied by repository code.

Only the supervisor receives the dedicated Pod-bound reporting token and
retained output. Reporting ingress validates TokenReview audience, Pod/Job
ownership and the immutable admission ConfigMap. Receipt acknowledges a durable
inbox record. Neither receipt nor broker storage grants task-outcome authority.
Repository commands remain inside the existing sandbox without reporting,
output or provider credential access.

Acceptance checks retained bytes and existing full-quality artifact/commit
bindings outside the write transaction. Inside one transaction it rechecks the
selected generation, request, writer, active attempt, deadline and current task
state. It commits the attempt/task projection, journal/outbox fact and terminal
cleanup intent together. Conflicting reuse of an identity or terminal slot fails.
Duplicate delivery does not repeat the transition. Stale and late valid reports
produce facts with `applied=false`; retained publication evidence remains
available without resurrecting cancelled or expired tasks.

## Progress, usage and publication

The supervisor samples its own progress, safe activity, usage and deterministic
publication journal every two seconds. It retains at most 256 changed advisory
snapshots per physical generation. Advisory delivery has a one-second timeout
and cannot stop coding. Missing or reordered progress does not block final
acceptance, and accepted budget counters cannot regress.

Snapshots include publication intent and the journal's PR receipt. The worker
continues to own push, draft creation, quality comments, remote-head checks and
uncertain-response readback. The final report includes independent execution,
quality and publication outcomes. Missing model measurements stay unavailable.
Final delivery uses its own immutable terminal slot, independent of advisory
delivery, and retries for up to 30 seconds after retaining the complete result.

Accepted coding decisions use the existing bounded outbox publisher and local
JetStream lifecycle stream on `orchestrator.coding.accepted.v1`. Broker outage
leaves accepted application outcomes intact. Shared backlog metrics include
coding facts. Stored advisory history becomes eligible for bounded pruning
only after its generation is unreachable and the 30-day history window passes.
Terminal observations remain retained for historical and publication recovery.

## Exit, cleanup and recovery

A final result can advance a task before the worker exits. Terminal state is also
the durable cleanup intent; subsequent reconciliation resumes cleanup after a
controller restart. Cleanup never changes the accepted execution/quality/
publication decision. Replacement and publication-only retry wait for observed
stop before selecting a new generation. Generation-specific reporting metadata
is excluded from the logical source-recovery identity, so recovery preserves
workspace, commits and consumed budgets without confusing different writers.

If the worker exits before HTTP receipt, recovery observes its original owned
Pod, Job and immutable admission, then adapts the retained terminal envelope
using its original identity. Missing or ambiguous Kubernetes ownership cannot
grant authority. Recovery never infers success from Job exit or unbound files.
A supervisor cancellation without durable task cancellation remains an execution
interruption and can resume retained work after stop confirmation.

Legacy file collection remains selected for migrated attempts. Recovery files,
final evidence and provider readback remain necessary. Their retirement requires
[the migration drain gates](event-platform-migration.md), including historical
reads and publication-only retries; zero running Pods alone is insufficient.
An older binary cannot read schema 17. Roll back with a compatible binary while
preserving the reporting selection of admitted executions.

## Validation

`dev/tests/coding-reporting.sh CONTROL_PLANE_IMAGE PINNED_WORKER_IMAGE WEB_IMAGE` uses
the explicit Docker Desktop context and the local Helm chart in a disposable
namespace. A real coding supervisor reports admission failure against absent
runtime metadata. The fixture holds cleanup to demonstrate terminal acceptance
and real JetStream acknowledgement before supervisor exit, then restarts the
controller and checks the persisted outcome and single receipt. Set
`AYNI_CODING_DELIVERY=retained` to keep ingress stopped through worker exit and
exercise retained-envelope recovery from the original owned Pod/Job instead.

This fixture does not establish managed-runtime compatibility, a successful
model session or a live GitHub publication round trip. Rust tests separately
cover transactional rollback, conflicting receipt replay, generation fencing,
legacy migration, cancellation/deadline races and reordered advisory reports.
Existing worker publication and recovery tests continue to cover deterministic
intent, remote-head conflicts, uncertain-response readback and retained budgets.
