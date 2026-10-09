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

Workflow installation RPCs, resources and fields were intentionally removed. Removed field numbers/names are reserved. Runtime reads expose structured findings, preparation phase and guarded rebuild availability. Upgrade clients and server together; see [cutover operations](../docs/operations.md).

Repository connections expose durable onboarding status, reason and inspected commit.
`AnalyzeRepository` retries analysis without granting runtime replacement.
The deployment-wide Ayni settings, release discovery, installation and service-test
API has been removed; clients must upgrade with this cutover.
