# Event reporting compatibility and recovery

Current reporting behavior is defined by the [event platform](../architecture/event-platform.md)
and its flow contracts. This guide records the compatibility gates for retained
generations and requirements for future reader retirement. GitHub owns delivery status.

## One authority during migration

Select the reporting contract and state-transition handler when admitting a new
execution generation. Retain that selection with its frozen inputs. Existing
digest-pinned workers finish under their original contract; changing the default
image does not upgrade admitted work.

During shadow evaluation, normalize legacy reports and new reports into the same
validation model. Compare their decisions without allowing the shadow handler
to schedule, publish, mutate accepted state or delete resources. Retain differences
with safe diagnostics. A single authoritative handler owns each execution.

Persist a stable event identity once for an adapted legacy result slot. Compare
validated contents to detect conflicts; do not mint another event on each poll.
Evidence hashes cover retained bytes. Protobuf serialization must not be treated
as a canonical representation for event identity.

## Compatibility invariants by flow

The implemented contracts define supported event names and payloads. Preserve
these invariants when changing readers or admitting a new reporting contract.

| Flow | Reporting responsibility | Preserve across upgrades |
| --- | --- | --- |
| Runtime verification | Receive retained report before Pod termination; accept frozen evidence; enqueue cleanup independently | Certificate, lock, source, platform and image bindings; source quality stays separate from runtime trust |
| Registry probes and image discovery | Typed access, missing-image and digest observations with retained diagnostics | Access failure cannot become image absence; owned probe cleanup follows collection |
| Official builder and trust preparation | Typed builder identity and trust evidence; reviewed proposal lifecycle | Current provider merge/readback checks, certificate selection and policy revalidation |
| Build, certify, publish and readback | Candidate and publication results with explicit provider observations | Engine remains through dependent publication; independent registry readback gates verification |
| Runtime activation | Commit Ready with valid current evidence and separate cleanup state | Selection/source guards; resources and credentials remain retained while depended upon |
| Coding activity and result | Bounded advisory activity plus independently ingested terminal evidence | Frozen budgets, independent execution/quality/publication outcomes, final commit bindings and stopped-writer checks |
| Model usage and task publication | Typed usage availability, publication intent/result and receipt milestones | No fabricated zero usage; remote-head checks, deterministic publication journal and uncertain-response readback |
| Connections and certificates | Accepted candidate/revision transitions plus explicit rejected replacements | Existing trusted operator access and Secret ownership; active revision survives candidate failure |
| Repository onboarding and projects | Accepted analysis, source sync, membership and removal facts | Frozen admitted tasks, idempotent mutations and separate logical removal/physical cleanup |
| Cancellation, deadlines and recovery | Durable decisions and cleanup intent can precede any worker result | Late results cannot resurrect work; observed remote side effects remain reportable; replacement waits for stop |
| Operator projections | Accepted outcome, execution, reporting, retry and cleanup displayed separately | Snapshot/cursor recovery, completeness, privacy and bounded activity |

Custom Runtime, input-answer/resume APIs and coordinated multi-repository work
are outside implemented coverage. Adding event envelopes does not implement those
features or move worker tool sequencing into the control plane.

## What retires, and when

| Retained mechanism | Event-selected behavior | Removal gate |
| --- | --- | --- |
| Full verification JSON in termination messages | Retained evidence reference and typed result ingestion | Every reachable legacy verification generation has drained; large-report tests pass |
| Trust outcome scraped from Pod logs | Retained typed trust result | Legacy trust generations and review continuations can resume; diagnostics remain accessible |
| Pod completion as the normal trigger for final collection | Authenticated final-result availability | Both result-before-exit and exit-before-result pass, including failed Job with valid evidence |
| File polling for normal coding progress/result detection | Reporting endpoint plus advisory delivery | Recovery scan remains functional; old pinned worker contracts are drained |
| Generic verification error as the only operator explanation | Typed failure class with safe retained detail | API/UI propagate classification and distinguish trust from reporting failures |
| Failure count incremented on each observation | Per-execution outcome accounting | Restart and repeated failed-Job observations count once |
| Implicit analysis-driven retry reset | Explicit retry/new-preparation intent | Operator actions work end to end and retained histories migrate truthfully |
| Ready waiting on unrelated deletion completion | Accepted Ready plus cleanup status | Admission/source guards and resource-dependency checks pass; blocked cleanup cannot activate stale evidence |
| Frequent full Kubernetes scans | Ownership-scoped watches and targeted reconciliation | Expired watch history, reconnect and missed notifications converge through repair scans |
| Redundant UI polling | Projection notifications and resumable resource streams | Connection/certificate/project coverage and snapshot recovery are complete; low-frequency repair remains |

Keep retained evidence and recovery files, resource snapshot streams, provider
readback, deadlines, periodic repair, Secret-reference retention and stopped-Pod
confirmation. Keep JSON logs and Prometheus metrics alongside OpenTelemetry.
Remove obsolete fields only after their API consumers and stored data migrate.

## Compatibility releases

| Release step | Behavior |
| --- | --- |
| Introduce | Add readers, schema migrations and shadow comparison; current handlers stay authoritative |
| Enable | Select new reporting for newly admitted generations; maintain legacy readers for frozen work |
| Drain | Stop creating legacy generations; enumerate active, resumable and retained legacy dependencies |
| Remove | Delete old collectors and API fields after compatibility evidence shows zero reachable dependencies |

Elapsed time alone does not satisfy the drain gate. Historical API reads and
retained publication retry can require old decoding after a Pod disappears.
Migrate historical representation or retain a versioned reader before removing it.
Never reinterpret a legacy failure counter as a precise execution count.

New fields should be additive under the existing Buf baseline. Use explicit
optional presence and unspecified enum values for unavailable information.
Reserve removed numbers and names. ProtoJSON changes need their own compatibility
checks. [Protobuf JSON compatibility](https://protobuf.dev/programming-guides/json/).

## Recovery and rollback

Pause new admissions for a broken flow, drain its active generations and switch
the default reporting contract for subsequent admissions. Existing generations
continue through their selected reader or retained recovery. Do not change
authority mid-execution or let two handlers perform provider actions.

Application rollback uses a compatible binary and retained data. A forward
database migration does not promise that an older binary can read it. Test a
backup/restore procedure with admitted work quiesced; do not restore a live older
snapshot that can repeat external side effects.

Broker outage leaves accepted decisions in SQLite and publication in the outbox.
Resume with original event identities; consumers deduplicate after the broker's
tracking window as well. A consumer behind expired replay history rebuilds its
projection from an authoritative snapshot and a matching cursor, not guessed events.

## Validation gates

Exercise these on the actual local worker topology before enabling each flow:

- Report above 4 KiB; retained diagnostics accessible after Job deletion.
- Result before process exit; process exit or loss before a final result.
- Repeated result, conflicting identity, stale generation and reordered advisory activity.
- Unauthenticated producer, forged source, wrong execution identity and repository-child reporting attempt.
- Restart after receipt, during acceptance, after publish and before acknowledgement.
- Broker outage, storage limits, consumer crash and unsupported schema quarantine.
- Result racing cancellation, deadline or selected-runtime supersession.
- Provider success with lost response; retry preserves one branch/draft/receipt.
- Blocked cleanup, engine dependency, retained credentials and stopped-writer replacement.
- Collector outage, malformed trace context and incomplete worker trace flush.
- Legacy pinned workers, historical reads and publication-only recovery across upgrade.

Use the pinned Rust checks, generated-binding checks, Buf compatibility, Helm
validation and managed-lock checks required by the affected change. Simulate
failures rather than treating successful unit mocks as cluster evidence.

## Operational acceptance

Expose result receipt/acceptance delay, outbox age, consumer lag, rejected events,
last worker activity, cleanup backlog and retained-storage usage. Record physical
executions separately from observation errors and reporting retries. Measure a
baseline and agree flow-specific latency and retention targets before production.

Add typed projection fields for accepted outcome, execution state, reporting
status, last meaningful progress, retry eligibility and cleanup condition. Show
safe reason, operation identity and retained diagnostics; link a trace when
available. A terminal result remains visible while execution is stopping.
No trace, heartbeat or broker acknowledgement establishes successful work.


## Retained compatibility boundaries

Normal event-selected verification uses authenticated result delivery. The trusted
Pod launch freezes a supervisor-only `VERIFICATION_REPORTING_AUTHORITY=events`
hint from the persisted authority. New workers omit the compact termination
reference only when that hint and the reporting admission are both present.
Shadow, retained and older workers keep their selected behavior. The admission
JSON remains unchanged so pinned workers with strict decoding can finish.
Safe termination failure codes remain diagnostic output.

Runtime-operation and coding collectors select their existing legacy readers
only for frozen work without event admission. Retained final-envelope recovery,
provider readback and observed stopped writers remain required. Review-only trust
continuations and publication-only retries can depend on legacy readers after
all Pods have stopped. Versioned stored-state decoding, historical API reads,
legacy failure accounting and evidence retention therefore remain supported.
The [drain inventory](operations.md#compatibility-drain-inventory) identifies these
dependencies without authorizing deletion.

Ownership-scoped Kubernetes watches wake guarded reconciliation. Repair scans,
deadlines, Secret references and resource-dependency observations still converge
when notifications are lost. Operator resource scopes use notification-driven
refresh with 120-second repair; task detail streams use the same repair interval.
Task lists and project task summaries keep 30-second discovery until a global
resumable task watch exists. Runtime notifications react to projection changes
without a separate one-second poll.
