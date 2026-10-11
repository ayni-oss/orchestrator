# CloudEvents for Orchestrator lifecycle events

Research snapshot: 11 October 2026. This recommends an envelope for the proposed
[event architecture](control-plane-events.md); no event transport or runtime
behavior is changed. Specification references use the released CloudEvents
1.0.2 documents, rather than the work-in-progress main branch.

Use CloudEvents for lifecycle facts exchanged between workers, the control plane
and independent consumers. It standardizes the common envelope while
Orchestrator defines each payload, its authority and its progression rules.
There is no additional CloudEvents server to deploy.

## Place in the CNCF stack

| Technology | Responsibility | CNCF status at research date |
| --- | --- | --- |
| CloudEvents | Event identity, context and representation | [Graduated](https://www.cncf.io/projects/cloudevents/) |
| NATS JetStream | Durable delivery, replay and consumer acknowledgement | NATS is [Incubating](https://www.cncf.io/projects/nats/) |
| OpenTelemetry | Instrumentation and export of diagnostic signals | [Graduated](https://www.cncf.io/projects/opentelemetry/) |
| Jaeger | Trace storage and investigation | [Graduated](https://www.cncf.io/projects/jaeger/) |

The database and retained evidence still own accepted state. CloudEvents does
not define workflow policy, event persistence, authentication or payload
semantics. The existing [authority levels](control-plane-event-flows.md#signal-levels)
remain necessary. [CloudEvents primer](https://github.com/cloudevents/spec/blob/v1.0.2/cloudevents/primer.md).

## Adoption scope

Start with worker result availability, controller acceptance/rejection, retry
exhaustion and cleanup lifecycle. Keep infrastructure observations distinguishable
from work results and accepted decisions. A worker can report available evidence;
only the control plane can announce its acceptance under current policy.

Commands retain their admission, authorization and idempotency contracts. A
`verification.requested` event records that an intent was persisted; it does not
establish that execution started. Do not automatically turn every RPC, state enum,
log record, heartbeat or browser snapshot into a lifecycle event.

Current protobuf APIs and the bounded `resource_events` projection remain
separate contracts. A CloudEvents envelope does not make their replay permanent
or replace attempt evidence. High-rate advisory activity has independent
retention from critical results.

## Proposed envelope

CloudEvents requires `specversion`, `id`, `source` and `type`. The released
1.0.2 specification uses `specversion: "1.0"`; it does not version our payloads.
The pair `(source, id)` identifies an event across retries.
[Core specification](https://github.com/cloudevents/spec/blob/v1.0.2/cloudevents/spec.md).

Use a stable logical producer `source`, a persisted UUID `id` and a qualified,
versioned `type`. Put resource identity in `subject`, occurrence time in `time`,
and a locally recognized payload schema identifier in `dataschema`. Distinguish
event schema version from resource revision and execution generation.

The following values and event type are illustrative proposals:

```json
{
  "specversion": "1.0",
  "id": "fdea456e-26a7-4ff6-9e36-2c1be9dc80bb",
  "source": "ayni://orchestrator/runtime-worker",
  "type": "io.ayni.orchestrator.runtime.verification.result.available.v1",
  "subject": "repositories/repo-42/runtimes/profile-9",
  "time": "2026-10-11T10:00:00Z",
  "datacontenttype": "application/json",
  "dataschema": "ayni://orchestrator/schemas/runtime-verification-result/v1",
  "operationid": "prep-17",
  "executionid": "exec-7",
  "causationsource": "ayni://orchestrator/control-plane",
  "causationid": "7ddfe4c5-f578-4cce-9324-370bb8ed18d8",
  "sequence": 8,
  "traceparent": "00-7431fc7927bb4f208986f2a4f2418dba-3f49c016c2df402b-01",
  "data": {
    "outcome": "verified",
    "evidenceid": "verification-report-72"
  }
}
```

These Orchestrator extensions are proposals:

| Extension | Meaning |
| --- | --- |
| `operationid` | Logical preparation or admitted operation |
| `executionid` | Physical execution generation used for stale-writer checks |
| `causationsource`, `causationid` | Source/ID pair of the triggering event |
| `sequence` | Producer order within this operation and execution generation |
| `taskid`, `attemptid` | Frozen task and logical attempt, where applicable |

Use lowercase alphanumeric extension names; place complex values and frozen-input
bindings in the typed payload. Envelope validity alone does not establish valid
evidence or current execution authority.
[JSON event format](https://github.com/cloudevents/spec/blob/v1.0.2/cloudevents/formats/json-format.md).

Keep evidence in retained storage with verified identity, hashes and frozen-input
bindings. The event references that evidence rather than carrying full Ayni
findings, tool output or repository content. Resolve schema/evidence identifiers
through controlled local mappings, never by blindly fetching producer URLs.

## NATS mapping

Initially send the whole event as UTF-8 structured JSON in the NATS payload,
matching the released [NATS binding](https://github.com/cloudevents/spec/blob/v1.0.2/cloudevents/bindings/nats-protocol-binding.md).
Define an explicit mapping from event family/type to NATS routing subjects.
For this example, a proposed routing subject is
`orchestrator.runtime.verification.result.available.v1`.

The CloudEvents `subject` identifies the affected resource; the NATS subject
controls delivery. Broker subscriptions do not automatically filter JSON
attributes. Preserve routing authorization separately from envelope metadata.
Modern NATS supports [headers](https://docs.nats.io/reference/protocols/client#hpub),
but the released CloudEvents binding defines structured mode; do not infer an
interoperable binary binding from header support alone.

Derive a stable `Nats-Msg-Id` from both `source` and `id`. Await JetStream's
publish acknowledgement before recording outbox delivery. Deduplication is
limited by the stream's tracking window, and a publish acknowledgement confirms
storage rather than application acceptance. [JetStream publishing](https://docs.nats.io/learn/jetstream/publishing/).

Persist consumer deduplication with accepted state and dependent outbox intents
in one transaction, then acknowledge consumption. Detect conflicting reuse of
event identity. Keep per-operation ordering and generation checks; a timestamp,
broker sequence or well-formed CloudEvent cannot replace those guards.

## OpenTelemetry correlation

Use the CloudEvents distributed-tracing extension: `traceparent`, with optional
`tracestate`. These identify the originating trace and remain stable when the
same immutable event is forwarded or replayed. Transport instrumentation can
record each delivery hop separately; it must not overwrite the event's
originating context. For a direct single-hop transmission, event and protocol
tracing context must agree when both are present.
[Tracing extension](https://github.com/cloudevents/spec/blob/v1.0.2/cloudevents/extensions/distributed-tracing.md).

Consumers create processing spans linked to creation context as described in
the [observability approach](observability-and-events.md). Record CloudEvent
identity on spans/logs and retain event causation independently. Trace context
is optional diagnostic metadata, not a deduplication key or execution fence.

## Integration boundaries

Use the official [Rust SDK](https://github.com/cloudevents/sdk-rust) for envelope
construction, validation and JSON representation where practical. Pin a tested
version; the SDK describes its APIs as unstable. Keep JetStream publication and
consumer acknowledgement explicit rather than assuming generic NATS send
success provides durable delivery.

Authenticate each producer and check its permitted routing subjects, types,
source and execution identity. Retain the trusted worker supervisor boundary;
repository code cannot announce controller acceptance. Preserve immutable event
identity on replay, independently retained evidence and consequential cleanup.

Before implementation, exercise duplicate delivery after the broker window,
same-identity conflicting evidence, unsupported schemas, stale execution,
broker outage, restart, malformed tracing context and cleanup backlog.
CloudEvents standardizes representation; those guarantees require our contracts
and tests. Requirements and delivery status belong in linked GitHub issues.
