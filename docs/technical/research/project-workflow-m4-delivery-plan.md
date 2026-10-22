# M4 delivery plan

Date: 2026-10-22. Status: agreed six-slice direction; individual PRs own delivery evidence.

M3 admits approved work without dispatching it. M4 takes an independently executable
task through isolated coding, Ayni quality evidence and reviewable draft publication.
Use the [execution preparation contract](../architecture/task-execution.md) for the
first slice and the [roadmap](../../../ROADMAP.md#m4-delivery-scopes) for milestone scope.

## Sequential delivery slices

| PR | Scope | Observable outcome and acceptance |
| --- | --- | --- |
| 1: Execution readiness and attempts | Define contracts; verify operator-provided local repository/source evidence, freeze the commit and lock digest, select a pinned runtime, authorize bounded execution budgets and freeze agent configuration. | One immutable prepared attempt or a retained blocked receipt. Exact retries, competing keys, wrong scope, transaction failure and retained-state recovery cannot duplicate attempts or dispatch work. |
| 2: Isolated coding execution | Isolated checkout/worktree and coding worker; durable dispatch, ownership, fencing, cancellation and retained results. | One task produces retained changes; stale workers cannot advance state after interruption or replacement. Enforce one active coding task initially. |
| 3: Ayni quality and evidence | Run supported Ayni commands on the actual worker topology; verify lock/runtime/platform identity, prepare dependencies, freeze final validation inputs and retain artifacts outside agent-writable storage. | Separate execution and quality outcomes with trustworthy completion accounting and evidence. A passing host check is insufficient. |
| 4: Draft publication | Commit/push accepted changes and create a draft PR under separate, narrowly scoped publication authority. | One reviewable draft; interruption and ambiguous responses recover without duplicate external actions or rerunning coding. |
| 5: Operator visibility and controls | Complete attempt, blocker, progress, evidence and separate execution/quality/publication views and bounded controls. | Operators can understand and inspect each outcome. Earlier slices expose basic visibility immediately. |
| 6: Cumulative M4 acceptance | Complete deployed interruption, cancellation, fencing, restart, cleanup and operational evidence. | Approval through draft publication passes on explicit Docker Desktop Kubernetes. Each preceding PR extends the same cumulative harness. |

Each PR starts from current origin/alpha after its prerequisite merges and targets
alpha. Runtime changes require pinned-toolchain CI checks, managed-lock checks and
affected behavior on Docker Desktop Kubernetes. Record actual source/image identities,
fixtures versus real integrations, and failed or blocked checks in the delivery PR.

## First-slice decisions

Repository verification starts with trusted operator-supplied local Git objects.
The verifier checks origin against the exact approved repository, resolves the
approved ref to a commit and hashes the lock blob at that commit. This proves
local source availability, not remote fetch/push permission or execution compatibility.
Prepared attempts must pass worker-side checkout and Ayni verification before later
execution/quality claims. No network credentials enter preparation requests.

Runtime selection uses an OCI digest pin. Compatibility, platform and dependency
preparation are verified in PR 3 using Ayni's own contracts; preparation does not
reinterpret lock policy. Configuration and numeric time/token limits are frozen
for the attempt; natural-language plan limits remain part of the reviewed task.

Attempt inputs and origin are immutable. M4 PR 1 permits one prepared attempt per
task, no coding dispatch and at most 32 preparation receipts per task. Unresolved
approved inputs remain blockers. Changing reviewed work requires a linked follow-up
plan; later recovery cannot silently rewrite an attempt.

## Scope boundaries and risks

M4 starts with serial task execution and isolated attempts. Dependency scheduling
and concurrent task execution belong to M5. Conversational attention and confirmed
recovery proposals belong to M6. Project assessment, MCP and more providers belong
to M7. User merge authority remains unchanged.

Automatic quality repair is not implicit in this plan: establish bounded repair
budgets and authorization explicitly before adding a repair loop. The largest
uncertainties are coding-worker integration, real Ayni compatibility on the chosen
worker topology and publication recovery. Each owning PR must settle those before
claiming delivery; this document does not promise a calendar estimate.
