# Decision 0011: project-centered UI and onboarding

> Historical record: task-pipeline and quality/publication ownership in this decision are superseded by the [worker-owned architecture](../architecture.md). They do not require compatibility code.
- Status: accepted
- Date: 2026-10-05
- Amends: Decision 0009 membership, initialization, and registry configuration.

## Projects and navigation

Projects is the landing page. The application sidebar contains Projects and
Configuration; task and repository pages sit beneath the project journey.
A project contains between one and 100 repository connections. Repositories may
belong to several projects. Optimistic concurrency protects membership edits.
Removal or archive preserves task history and existing work but prevents new
tasks and retries for removed members. Re-adding a repository restores admission.
A task targets one explicit member repository. Legacy native callers may omit
that selection only when the project has exactly one member.

Runtime setup and the image destination are repository-owned and shared by every
project using that connection. Editing membership never initializes a runtime
or mutates repository content. The UI explains the shared configuration boundary.

## Approved onboarding target

Selecting a repository will queue durable inspection immediately. Results and
approval survive closing the creation dialog. Initialization is a separate,
explicitly approved operation bound to source and proposed scope; changed inputs
require renewed approval. A bootstrap environment lets the agent propose missing
Ayni configuration, CLI-generated locks, and integrations through setup PRs.
Repository-owner review establishes producer trust. Never merge automatically.
After merge, reinspect and build or verify a certified environment.

Credential revisions activate after validation without restarting the control plane.
Admin references remain read-only in the UI; UI-managed credentials can be replaced,
tested, disabled or deleted when unreferenced. Labels describe ownership and do not
restrict Kubernetes RBAC.

Configuration separates source-control Apps from container registry connections.
Support GitHub App creation/import, installation discovery, rotation and disabling.
GitHub App private keys and registry tokens share the same storage model.
Support one configured App and multiple GHCR connections. Registry forms accept
username, push token and a separate pull-only token; store credentials
in namespace Secrets, with safe references and metadata in the database. Scoped
workers test authentication, pull and push independently using a temporary
artifact with observable cleanup. Validate destinations and authentication
challenges before forwarding credentials. One registry and image namespace is
selected per repository. Candidate workloads never receive signing or push access.

## Approved task target

Freeze one repository and an ordered set of supported profile revisions and image
digests. Ayni Managed runs before Custom Runtime against shared candidate work.
Unsupported profiles are visible and skipped. Stop coding on failure or requested
input. Verify the final commit independently in all participating runtimes, then
publish one draft with truthful results and bounded signals. Cancellation prevents
new publication. Evidence remains outside agent-writable storage.

Task details show overview, activity and results, stacking on mobile. Technical
identifiers and immutable evidence remain available in collapsed details.
Project summaries show unavailable statistics explicitly rather than zero.

## Current implementation boundary

The project foundation implements shared editable membership, task ownership,
repository selection, the sidebar, project cards, validated creation and
project-scoped task dialogs. Source-control onboarding supports one GitHub App,
manifest creation/import, validated key rotation and explicit administrator
references. GHCR connections use namespace Secrets, scoped probe Jobs and
repository-owned image destinations; credential revisions reconcile without
restarting the control plane. Credential management is restricted to the trusted
operator listener. SQLite stores safe metadata and references, not credentials.

Approval/bootstrap initialization, multi-profile execution and summary
projections remain separate capabilities. Existing task admission freezes one
ready runtime. Custom Runtime remains unsupported. Historical task-pipeline
ownership above remains superseded by the worker-owned architecture.

The current fresh-install boundary is schema version 6. Earlier databases are
rejected; no migration or automatic deletion is performed. Schema version 4 was
the original project-foundation boundary recorded by this decision. Project
protobuf APIs retain single-member creation for native callers and repeated
creation/update fields. An empty update membership list preserves membership;
nonempty replaces it. Use isolated new storage for validation and the exact
retained-data cutover procedure in [operations](../operations.md).

## Runtime overview refinement (2026-10-07)

The accepted repository detail page now serves as its runtime overview. Its header
contains repository identity, the GitHub link, connection status and default
branch. Each runtime owns its lock information, image destination, readiness and
configuration dialog. Inspection events use transient notifications; ongoing
progress and failures remain visible on the affected runtime.

Registry connection and approved image namespace selections now belong to the
logical runtime profile and survive source updates and rebuilds. This refines the
repository-owned destination described above; it remains shared across projects
using that runtime. Existing schema-6 profiles without a runtime selection inherit
the legacy repository default until edited. The additive metadata requires no
state reset. Destination changes invalidate that runtime's admission authority
until verified, while already admitted work retains its frozen destination and
credential revision. See the current [architecture](../architecture.md) and
[onboarding flow](../operator-onboarding.md).
