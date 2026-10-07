# Private worker-owned API

The canonical API is `ayni.orchestrator.v1`. This private breaking replacement
uses the `worker-owned-v2` baseline recorded in `BASELINE`. Clients must regenerate
bindings and upgrade with the server; there are no compatibility readers or
migration adapters for the superseded task pipeline.

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
