# Repository preparation and workspace delivery

Status: connection foundation delivered in source; full catalog, preparation and cumulative qualification remain pending. See [integration connections](../architecture/integration-connections.md) for the implemented boundary.

Deliver the [roadmap focus](../../../ROADMAP.md#next-focus-repository-preparation-with-a-usable-ui)
through the [shared repository preparation contract](../architecture/repository-preparation.md).
Each slice includes canonical v1 APIs, PostgreSQL authority, UI, safe diagnostics
and local acceptance. Each PR records its own delivery status and evidence.

## Usable increments

| Slice and prerequisite | Browser outcome | Required acceptance |
| --- | --- | --- |
| Shared catalog and project association; existing hierarchy APIs | Open the new shell, configure source connection references, connect a repository and select it from a project. Reuse that repository from a project in another workspace. | Stable provider identity, no workspace-membership prerequisite, scoped associations, rejected unauthorized use, bounded discovery and exact response-loss retry. |
| Frozen inspection and setup; catalog slice | Analyze an exact commit and see Ayni inputs, supported tooling and corrective findings. Configure registry/certificate references, inspect probes and authorize a trust proposal when needed. | Missing/malformed/unsupported inputs block progress; disabled Ayni stays disabled across restart; probe failures stay distinct; duplicate/closed/conflicting trust proposals retain explicit recovery; merged trust is revalidated. |
| Existing runtime verification; setup gates | Inspect discovery, exact image digest, verification evidence and ready/missing/invalid/unavailable states. | No build on outage or denial; reject wrong platform, lock, signer, protected content or stale artifact; source quality failure cannot be mistaken for invalid environment provenance. |
| Runtime production; verified contract and qualified build topology | Enable bounded preparation, follow build/publication/readback/verification/cleanup and explicitly request replacement. Inspect retained failed operations and retry decisions. | Real Ayni build and registry pull; separate credentials; guarded publication; uncertain push readback; engine/workload loss, exhausted allowance, stale reports, cancellation and cleanup restart. |
| Conversation to delivery; artifact admission and trusted source service | Discuss a goal, review/approve a plan, prepare and explicitly start a task, then inspect patch, quality and authorized draft publication. | Project association and approved source cannot be substituted; server verifies source/lock/upstream ancestry; attempt freezes artifact evidence and budgets; no local source-pack command; later selection changes leave the attempt unchanged. |
| Feedback and qualification; complete journey | Answer worker questions, confirm exact recovery and follow attention to continued outcomes. Navigate from task to shared preparation and retained traces/evidence. | Cross-workspace reuse, independent conversation scope, restart/replay, stable evidence navigation and separate coding/quality/publication outcomes in one cumulative deployed journey. |

Rows are dependency order, not a requirement for one large PR per row.
Split rows into smaller end-to-end PRs when needed, while keeping each increment
testable through its UI. Begin tracing, durable history and safe failure display
in the catalog slice; do not defer them to final qualification.

## Product interface

| Surface | What the maintainer can inspect and do |
| --- | --- |
| Shared integrations | Inspect source, registry and certificate status; configure administrator-owned references and test supported capabilities. Read APIs never return credentials. |
| Repository catalog/details | Connect once, opt into Ayni preparation, analyze, inspect the frozen commit and findings, select setup, view history and request permitted retries. |
| Runtime inspector | Compare desired revision with verified artifacts, inspect stages/timing/cleanup and review explicit rebuild or trust decisions. |
| Workspace/project | Create and navigate projects; select catalog repositories directly and inspect linked preparation blockers. |
| Chat, Plans and Tasks | Discuss, review the exact version/digest, authorize actions and inspect accepted outcomes plus provisional activity. |
| Evidence/recovery inspector | Follow immutable origin, questions, answers, allowances, attempts, quality and draft references without changing execution authority by reading. |

Design from current user journeys and the retained web example, then implement
against canonical APIs. The historical BFF and client contracts remain excluded.
Initial qualification targets the trusted local operator and explicit Docker
Desktop topology; the UI must not expose private listeners to remote users.

Administrator deployment, GitHub App installation, Secret provisioning and the
initial qualified builder topology may require external setup. Document those
prerequisites before each test journey. Once configured, repository analysis,
runtime preparation and task source handoff run through the product without
manual database writes or terminal preparation of each task.

## Cross-cutting browser gates

- Reload/reconnect reconstructs state from retained reads; closing the browser does not cancel accepted work.
- Persist exact pending-command scope, identity and bytes for interrupted retries; never silently generate another action.
- Concurrent tabs detect stale selections and approvals rather than overwriting them.
- Scope switches fence late responses; project views cannot display another project's private data.
- Every waiting/failure state shows the current stage, evidence, safe reason and permitted next action.
- Incremental text remains provisional until canonical acceptance; read failures retain loaded evidence and offer retry.
- Selecting or inspecting an artifact, finding or recovery record cannot authorize execution or publication.

## Runtime and recovery matrix

| Case | Required invariant |
| --- | --- |
| Duplicate connect/analyze/build command | Recover one accepted operation and its receipt; changed bytes under the same key conflict. |
| Source, registry, certificate or platform changes mid-operation | Preserve frozen history; fence old activation and require applicable new verification. |
| Confirmed absence versus unavailable registry | Only absence permits opted-in automatic build. Invalid existing content needs explicit replacement authority. |
| Publication response lost or process restarted | Read back the expected candidate before retry; no blind replacement or duplicate external action. |
| Owner restart or workload loss | Adopt only the exact owned workload or retain an interruption; replacement uses a new authorized generation and bounded allowance. |
| Cancellation or deadline | Fence late reports and complete owned cleanup; retain uncertainty about external effects. |
| Cleanup failure | Expose pending cleanup and resume it after restart; never report ready prematurely. |
| Several projects sharing a repository | Reuse artifact evidence without sharing transcripts, task authority or credentials; prevent duplicate competing preparation. |
| Association/catalog removal | Check active operations and retained references; retain provenance and require separate authority for external deletion. |
| Task runtime becomes stale or unavailable | Retain the frozen selection and report a blocker/failure; never substitute the latest tag or another image. |

## Evidence and completion

For every delivered slice, record the committed source revision, qualified tool
and image identities, explicit cluster context, prerequisites, commands, retained
receipts/artifact digests, browser results and failed or blocked checks. Keep
evidence outside repository-controlled writable storage and exclude secrets and
raw sensitive content from exported telemetry.

Use Docker Desktop Kubernetes with the explicit `docker-desktop` context and
extend the existing cumulative harness. Restore browser coverage against the new
UI before advertising support; retired browser evidence cannot qualify current
source. Mock pages and deterministic coding fixtures establish neither actual
runtime compatibility nor live-provider delivery.

The focus is complete when a maintainer can connect a real authorized repository,
inspect its committed Ayni setup, build or reuse and verify its image, select it
from projects in two workspaces, and deliver approved work through managed quality
to a reviewable draft. The same journey must expose questions/failures, confirmed
recovery and retained causal evidence across browser and workload interruption.
Report live-provider/publication qualification separately from local protocol
fixtures; record any external authority required for those tests.

Production capacity, remote tenancy, arbitrary runtime types, automatic policy
bootstrap, merge and deployment remain separate scope. A successful local journey
does not establish those capabilities.

## Related contracts

- [Project workspace](../architecture/project-workspace.md) records present UI status and the next interface boundary.
- [Local development](../operations/local-development.md) defines deployment and retained-resource operations.
- [Operational acceptance](../operations/operational-acceptance.md) and [observable acceptance](../operations/observable-execution-acceptance.md) define existing evidence limits and currently unavailable browser profiles.
