# Repository preparation and workspace delivery

Status: source connections, shared catalog with workspace availability and project subsets, static Ayni
inspection and profile-gated Kubernetes runtime preparation are implemented.
Server-side task handoff and cumulative live source/registry qualification remain pending. This plan details
[roadmap milestone A](../../../ROADMAP.md#a-workspace-scope-and-execution-ready-repositories);
[the alpha contract](../architecture/alpha-product.md) owns product scope.

## Scope and ownership

Keep repository identity and runtime artifacts in the shared catalog. Workspace settings controls
availability, and projects select only the repositories needed for their
work from that set. Several workspaces may reuse the same catalog repository and
verified artifact without sharing project-private context, credentials or grants.

Migration 0040 preserves existing project associations by adding their repositories
to workspace availability. Repositories attached to a project cannot be removed
from workspace availability. The [repository contract](../architecture/repository-preparation.md)
owns source/runtime gates and the Ayni/Orchestrator compatibility split.

## Delivery increments

| Increment | Browser outcome | Required evidence |
| --- | --- | --- |
| Workspace availability and project subset | Make catalog repositories available to a workspace; choose a project subset; reuse a repository in another explicitly configured workspace. | Stable identity, rejected out-of-workspace selection, context isolation, exact lost-response retry, removal/active-work rules and a safe transition for retained associations. |
| Frozen inspection and setup | Inspect one exact commit, understand policy/lock/tool/trust findings and finish required setup. | Supported Ayni contracts, bounded source reads, explicit missing/invalid inputs, retained access failures, reviewed trust changes and fresh inspection of the accepted source. |
| Reuse or produce a verified runtime | Select an admitted profile, registry, signer and allowance; inspect discovery, build/certification/publication, verification and cleanup. | Actual source acquisition, Ayni build, registry push/readback/pull, separate credentials, wrong-image/trust rejection, uncertain publication recovery and completed cleanup. |
| Server-side task handoff | Select a ready repository/runtime from a project and prepare the reviewed task without local source-pack commands. | Server verifies scope, Git objects, original lock, upstream ancestry and artifact evidence; attempts freeze exact source/runtime/configuration and allowance. |
| Connected task checkpoint | Follow one admitted task to isolated execution, retained quality and authorized PR delivery in the same product. | No substituted source/artifact, no duplicate effects on reconnect/restart, explicit blockers, retained evidence navigation and an actual browser/API/worker journey. |

These are increments, not a requirement for one large PR per row. Implement the
canonical API, UI and focused local acceptance together. Research/planning may
proceed while runtime preparation is blocked; execution waits for its required
readiness evidence. Continue through the roadmap's engineering-loop and DAG
milestones before claiming alpha complete.

## Runtime and recovery requirements

| Case | Required behavior |
| --- | --- |
| Registry outage, denial or rate limit | Retain the failed stage and next action; never infer image absence or authorize a build from an unavailable observation. |
| Confirmed image absence | Consume an explicit bounded preparation grant; invalid existing content requires replacement authority. |
| Source/registry/signer/platform changes | Preserve the frozen operation and fence stale activation. New inputs require applicable new preparation. |
| Publication response lost | Read back the exact intended candidate and identity before another external action; no blind overwrite or duplicate publication. |
| Workload loss or owner restart | Adopt only exact owned workloads; retain interruption, generation and allowance decisions. |
| Cancellation or exhausted allowance | Fence late reports, retain external-effect uncertainty and confirm owned cleanup. |
| Cleanup incomplete | Show pending cleanup and resume it; never label the artifact ready prematurely. |
| Workspace/project selection removed | Check active work and retained references; fence new admissions without rewriting frozen attempts or deleting shared catalog resources. |
| Artifact stale or unavailable | Keep the attempt's exact selection and report a blocker; never substitute a moving tag or unrelated image. |

The environment-changing task path is specified in the
[alpha contract](../architecture/alpha-product.md#environment-changing-work).
Preparation must support retained candidate source/lock revisions without
weakening signing, independent verification or final managed-quality boundaries.
Do not advertise changed-lock execution until that complete path is qualified.

## Browser and evidence gates

- Reload/reconnect reconstructs retained state. Closing the browser does not cancel accepted work.
- Interrupted retries reuse the exact command scope, bytes and identity. Changed inputs cannot reuse the same command identity.
- Concurrent tabs reject stale selections; scope changes fence late responses and prevent another project's private data appearing.
- Show the stage, waiting reason, allowance, safe failure and permitted next action. Provisional output cannot establish completion or grant authority.
- Trace source, runtime operation, artifact, plan, task, attempt, provider/tool activity, quality and PR relationships without exporting credentials or sensitive raw content.

For each delivered increment, record committed source, explicit deployment/context,
qualified tool/image identities, prerequisites, commands, browser results, receipts,
artifact digests and failed/blocked checks. Evidence remains outside agent-writable
storage. Fixture success is separate from actual provider, registry and forge
qualification; historical browser coverage does not validate current source.

The default Docker Desktop preparation profile remains disabled after a failed
network-policy negative control. The separately authorized Cilium profile has a
measured Node component and supervisor lifecycle envelope. Use the
[runtime packaging evidence](../../../deploy/images/runtime-preparation/README.md)
for exact CLI/profile prerequisites and remaining live integration gaps. Never
fall back to another current cluster or imply an unqualified runtime is supported.

Initial administrator deployment, App installation and signer/profile setup may
require local tools. Once those prerequisites are configured, repository/runtime
preparation and task source handoff must run through the product without manual
database writes or terminal preparation for every task.

## Completion boundary

This milestone finishes when a maintainer can configure workspace availability,
select a project subset, inspect a real Ayni-enabled repository, reuse or build
and publish its verified runtime, and hand exact inputs to an isolated task.
Reuse in another workspace requires that workspace's own availability selection.
The connected task checkpoint establishes the handoff; complete research,
autonomous repair, DAGs and flexible PR layout remain required by the wider alpha
roadmap, not claims of this setup milestone.

Run focused local browser, PostgreSQL, JetStream and deployed scenarios for affected
behavior. PR CI remains lean; this plan does not add end-to-end CI gates. Use the
[local operations guide](../operations/local-development.md) and preserve retained
resources during restart tests. Destructive cleanup, external release publication,
merge and deployment are separate authority.
