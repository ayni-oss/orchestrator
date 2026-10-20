# Private orchestrator API

The canonical API is `ayni.orchestrator.v1`. This private breaking replacement
uses the `repository-builders-v4` baseline recorded in `BASELINE`. Clients must regenerate
bindings and upgrade with the server. Schema-six/seven databases migrate without
retaining workflow execution compatibility; see [release state](RELEASES.md).

Connectors, projects, repositories and runtime profiles retain their resource
behavior. Development tasks freeze source, runtime, model, constraints and
execution limits at admission. Attempts own worker progress, results, model
usage and pull request receipts. Execution, quality and publication outcomes are
independent. Quality counts are optional: absence means unavailable.

`DevelopmentTaskService` supports task admission, list/get, cancellation,
ordinary retry, attempt list/get, publication-only retry and resumable updates.
`RetryTaskPublication` resumes retained final work without coding again. Ordinary
retry creates a new attempt. There are no standalone candidate or review
resources and no separate evidence-reference service.

Generate all bindings and descriptors with the repository tooling. Validate
protocol fixtures and the explicit breaking baseline before release. After the
replacement baseline is established, routine changes must obey the configured
Buf compatibility checks. Bundles identify an immutable private source revision;
clients verify checksums and regenerate before upgrading. Bundle creation does
not publish a public endpoint or package.

`CertificateService` is available only on the trusted operator listener. Import and
replacement requests contain write-only signing seeds; responses expose public key,
revision, ownership, status, usage and safe Secret references. Mutations require
idempotency keys and, for existing resources, expected resource versions.
`RuntimeProfileService.SetRuntimeCertificate` selects a certificate for an Ayni
runtime; an empty ID follows the global default. Profile reads expose the selection
and effective safe certificate metadata. Custom runtimes reject certificate
assignment. Signing seeds never appear in runtime profile or frozen task DTOs.

Workflow installation RPCs, resources and fields were intentionally removed. Removed field numbers/names are reserved. Runtime reads expose structured findings, preparation phase and guarded rebuild availability. Upgrade clients and server together; see [cutover operations](../docs/technical/operations/operations.md).

Repository connections expose durable onboarding status, reason and inspected commit.
`ConnectGithubRepository` and `CreateRepositoryConnection` accept optional
`use_ayni`; explicit false persists the opt-out and skips automatic analysis and
managed runtime creation. Absent values retain legacy enabled behavior.
Repository reads expose the saved choice. `AnalyzeRepository` explicitly enables
Ayni and retries analysis without granting runtime replacement.
`RemoveRepository` is restricted to the trusted operator listener and requires
an idempotency key and expected repository resource version. It atomically removes
unused repository onboarding and runtime state, preserves action receipts, and
emits repository/profile removal updates to resumable watches. Projects, retained
tasks, and active runtime preparation block removal. Failed operation cleanup
remains durable until the runtime reconciler completes it. Provider content,
registry images, integrations, and certificates are not removed.
The deployment-wide Ayni settings, release discovery, installation and service-test
API has been removed; clients must upgrade with this cutover.

`RepositoryService.ListRepositorySummaries` and `ProjectService.ListProjectSummaries`
provide complete product overview projections. Runtime readiness requires at least
one operational profile for every project repository, with every such profile ready.
Unsupported profiles do not affect readiness. Counts include every operational
profile, rather than the first page of profiles. Project summaries include the latest
created task across retained history; equal creation times are ordered by task ID.
An absent latest task with `tasks_complete=true` means there are no tasks.
Readiness completeness and task completeness are explicit fields. Storage errors,
corrupt resources and result-bound failures fail the RPC rather than returning
partial success. Standard pagination freezes the entire summary projection;
`snapshot_cursor` is the corresponding service cursor, not a cross-service watch
cursor. Refresh by starting a new first-page query.

`DevelopmentTaskRequest.coding_turns` optionally selects 1–128 total model turns
for implementation and quality repairs. Admission freezes it into
`DevelopmentSnapshot.limits.coding_turns`; clients omitting it retain the deployment
default. The task form always submits an explicit task budget, defaulting to 24.

## Verification lifecycle payloads

`ayni.orchestrator.events.v1` is an additive lifecycle payload package, separate
from the resource API. Rust uses generated Prost/ProtoJSON codecs and the
CloudEvents SDK in `backend/libs/events`; TypeScript bindings use the same schema.
Shared fixtures in `testdata/verification` cover both languages. See the
[verification event contract](../docs/technical/reference/verification-events.md) for authority, wire
validation and recovery boundaries. Authenticated receipt is opt-in and separate from acceptance. Authority is frozen
per preparation: retained/shadow generations keep the reader, and opted-in event
generations accept authenticated receipts. `VerificationDecisionRecorded` carries the accepted runtime
report decision and aggregate revision. Its CloudEvent is stored atomically with
the runtime projection in the journal/outbox; broker publication comes later.
Resource bindings accept the resource API’s typed UUID identifiers as well as
bounded simple identifiers used in fixtures.

## Coding lifecycle payloads

`CodingReportAvailable` binds retained progress/final snapshots to a physical
writer generation and exact worker request. `CodingDecisionRecorded` includes
causation, aggregate revision and whether the observation changed task state.
See [coding lifecycle reporting](../docs/technical/reference/coding-events.md) for authenticated
ingestion, independent outcomes, writer fencing and retained-envelope recovery.

## Conversation workflow payloads

The canonical `ayni.orchestrator.events.v1` workflow messages define the versioned
application-to-executor command, signed executor report, and targeted application
decision payloads for the replacement conversation runtime. Every encoded workflow
CloudEvent requires an RFC 3339 produced time and canonical W3C `traceparent` in
addition to its immutable source, event ID, scope, assignment, generation, causation,
and correlation bindings. Failed reports and rejected decisions carry stable
lowercase reason codes, never raw exceptions or secret-bearing diagnostics.

The payload schema and cross-language signing fixture are compatibility commitments.
The current executor ledger, PostgreSQL watch service, and JetStream configuration
remain feasibility artifacts and are not registered as production workflow owners.
See the [M1 feasibility evidence](../docs/technical/research/project-workflow-m1-feasibility-evidence.md)
for reproduction commands and open gates.

## Operator lifecycle and watch contracts

`LifecycleDecisionRecorded` records a committed connection, certificate, project
or repository transition with its resource revision, projection cursor, digest and
safe classification. It uses dedicated journal/outbox storage and the existing
accepted-fact delivery mechanism. It cannot authorize a mutation through broker replay.

`WatchConnections`, `WatchCertificates` and `WatchProjects` accept an optional
`after_cursor`. A fresh watch sends current resources and an explicit
`snapshot_complete` boundary at the matching service cursor, including for empty
snapshots. Updates carry their cursor and deletions set `deleted`. An expired
cursor requires a fresh snapshot; cursors cannot move across services. Connections
and certificates remain trusted-operator APIs. See
[operator lifecycle facts and streams](../docs/technical/reference/operator-projections.md).
