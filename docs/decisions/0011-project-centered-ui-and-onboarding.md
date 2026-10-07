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

Configuration separates source-control Apps from container registry connections.
Support GitHub App creation/import, installation discovery, rotation and disabling.
Registry connection forms accept HTTPS endpoint, username and token; store tokens
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
project-scoped task dialogs. Queued inspection, approval/bootstrap initialization,
dynamic connector credentials, registry tests, multi-profile execution and summary
projections are separate delivery work. Existing task admission still freezes one
ready runtime. Custom Runtime remains unsupported.

Schema version 4 is a fresh-install boundary. Earlier databases are rejected;
no migration or automatic deletion is performed. The protobuf additions retain
single-member creation for native callers and add repeated creation/update fields.
An empty update membership list preserves membership; nonempty replaces it.
Use isolated new storage for validation.
