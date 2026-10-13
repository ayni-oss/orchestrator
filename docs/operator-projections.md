# Operator lifecycle facts and resource streams

Connections, certificates, repository onboarding and projects retain accepted
controller facts alongside their resource projections. A committed fact records
the resource identity, revision, projection cursor and projection digest. It uses
`build.ayni.orchestrator.lifecycle.accepted.v1` and dedicated lifecycle
journal/outbox tables with the existing delivery mechanism. Broker delivery never
authorizes a resource mutation.

## Authority and privacy

Resource mutations retain their existing revision checks, idempotency receipts,
provider validation and Secret ownership rules. Changed connection observations
can share the desired resource version; their durable stream cursor orders the
public status changes. An unchanged projection emits no additional fact.

A failed connection replacement preserves a usable active revision. Its public
failure and probe/cleanup fields remain visible independently of that revision's
readiness. Certificate default changes publish both affected certificates in the
same transaction. Deletion publishes a tombstone before removing the projection.
Frozen tasks and preparations continue to retain their original dependencies.

Coding attempts persist reporting state separately from their execution and
publication outcomes. Only an applied observation marks event reporting accepted;
ignored late observations cannot change it. Cleanup moves independently through
pending, stopping and complete, with each changed projection available to task
watches. Older retained attempts expose unavailable status until observed.

Lifecycle envelopes carry a digest and bounded public classification, not
credentials or the write request. Connection and certificate streams expose the
same safe metadata as their trusted-operator reads. Those services are absent
from the custom-client listener.

## Snapshot and replay

`WatchConnections`, `WatchCertificates` and `WatchProjects` provide a complete
initial snapshot followed by `snapshot_complete` at its matching service cursor.
The completion marker also appears for an empty snapshot. Preexisting operator
resources appear even when they predate resource-event publication.
Snapshots use the existing 10,000-resource and 8 MiB read limits; exceeding a
limit returns an explicit error instead of a partial snapshot.

Clients resume with `after_cursor`. Updates and tombstones advance the service
cursor; a cursor outside retained history requires a fresh snapshot. Clients
retain cursors per service because replay and retention bounds are service-specific.
Resource-event replay is separate from accepted-fact retention.

The browser shares one EventSource across its five operator resource scopes;
notifications identify the changed scope and invalidate existing loaders without
carrying resource or credential payloads. On reconnect, each upstream watch takes
a fresh snapshot because independently versioned scopes cannot share a replay
cursor. Dedicated scoped subscriptions retain cursor replay. Each scope keeps
a 120-second repair timer. Sharing the browser connection leaves HTTP/1.1 slots
available for navigation and the separate task stream.

Deferred updates refresh within one second after editing or hidden state ends.
Background refresh also pauses during submissions and navigation so a lifecycle
update cannot cancel a redirect. Mutation-triggered refresh uses the same
coordinator and can refresh explicitly after saving.

Task details retain their stream through reporting and cleanup. Task lists keep
their existing repair interval; project task summaries have a 30-second repair
subscription. Repository onboarding retains its own stream cursor independently
from runtime-profile updates.

Task attempt projections expose the retained producer generation's reporting
state and cleanup state independently from execution and quality/publication
results. Older attempts with no recorded state display unavailable. A retry of
execution and a retry of publication remain separate operator actions.

Runtime cards expose their existing settings dialog through the card action menu.
Menu scroll locking is disabled because restoring a whole style attribute is
blocked by the application's CSP; dialogs retain the existing CSSOM scroll lock.

## Kubernetes observations

Namespace-scoped metadata watches observe owned runtime and coding Jobs/Pods,
build-engine Pods and UI-managed credential Secrets. They retain no object cache
or Secret payload. Paginated relists bound response size; stream interruption or
expired history starts a fresh list/watch and wakes reconciliation even when
the new list is empty.

Notifications coalesce into runtime, coding and connection reconciliation lanes.
Each lane serializes work and rate-limits observation-driven passes. Existing
periodic repair, provider readback, deadlines and stopped-writer confirmation
remain authoritative. Unlabelled administrator Secrets use periodic readback.

The connection role requires Secret metadata list/watch, and the runtime role
requires Job/Pod list/watch in the configured namespace. Kubernetes RBAC does not
restrict list/watch grants by the client label selector; the controller applies
ownership selectors to its requests. Observation failure leaves repair active.

## Compatibility

SQLite schema 18 adds lifecycle journal/outbox storage and preserves schema-17
verification, runtime operation and coding facts. Existing generations retain
their selected readers. Older binaries cannot open the upgraded database.
Legacy collector and polling retirement remains governed by
[the migration gates](event-platform-migration.md#what-retires-and-when).

See the [protocol contract](../proto/README.md),
[local development](local-development.md) and
[event platform implementation proposal](event-platform-implementation.md).
