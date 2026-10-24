# Isolated task delivery acceptance

M4 takes one approved independent task through isolated coding, managed quality
and a reviewable draft. Run the cumulative gates below before claiming milestone
completion. Each delivery PR records its executed evidence and failed or blocked
checks; this matrix defines the requirements.

## Run the complete profile

Use a new acceptance namespace and a new absolute evidence directory. Run the
browser conversation suite in a separate deployment; its session PVCs would
invalidate this profile's retained-volume comparison.

```sh
export AYNI_ORCHESTRATOR_NAMESPACE=ayni-orchestrator-acceptance-m4
dev/ctl quality-runtime
AYNI_ORCHESTRATOR_TASK_FIXTURE=true dev/ctl deploy
dev/ctl check
pnpm exec playwright install chromium
dev/ctl acceptance /absolute/new/m4-evidence --m4
```

The command requires the explicit `docker-desktop` context. It extends the same
quality/publication harness used by earlier slices, then uninstalls and reinstalls
the tested release with the same image references and publisher binding. Cleanup
preserves authoritative data, application credentials and worker storage; it
never invokes purge or repairs database rows.

`acceptance.json` is written only after all proofs pass. It names each case and its
evidence files, source revision/manifest/diff, actual workload images, managed
runtime pin and external-action counts. `outcome.json` records failure if any
command or assertion fails. Keep the complete evidence directory, including
`quality/`, browser screenshots and cleanup inventories.

## Cumulative matrix

| Boundary | Required observation | Executable proof |
| --- | --- | --- |
| Approval and preparation | Exact reviewed version admits one task set; source/lock/runtime/configuration/budgets and immutable origin survive response loss, races and restarts. | PostgreSQL `plan_approval`/`task_execution`, API/browser tests, `quality/plans-*` and `cleanup-preparation.txt`. |
| Isolated execution | A serial owned task reconstructs the exact source snapshot and retains accepted changes. Supervisor restart adopts the same Pod; Pod loss does not retry coding. | PostgreSQL `task_runs`, task filesystem/ledger tests, `quality/task-supervisor-active-restart.txt` and `quality/task-pod-loss-result.txt`. |
| Cancellation fence | Keep the actual old coding worker alive after cancellation. Its delayed result cannot advance the new generation; restoring the supervisor releases the slot without accepting that result. | `quality/task-fence-*`: report-retry exit 1, advanced generation, empty accepted patch and exact original start receipt. |
| Managed quality | Passing and failing checks execute in the selected Ayni runtime with frozen inputs, trustworthy target completion and retained artifacts. Failed quality preserves coding completion and cannot authorize publication. | Managed worker/runtime tests, `quality/task-quality-*`, `failed-quality-rejected.txt` and cleanup quality replays. |
| Credential and source isolation | Independent worker PVCs, credential-free prepare/quality containers, read-only validation source, separate artifact mounts and publisher-only GitHub credential mount. | `task-topology.json`, Pod specification tests and Helm checks. No Secret contents are inspected. |
| Reviewed publication | UI displays canonical metadata and evidence digests, requires consent, and recovers an accepted response after reload with the same command/receipt. | `console/outcome.json`, desktop/mobile screenshots and cumulative console tests. |
| Branch push before create | Interrupt the publisher after the deterministic branch is pushed, before consuming create authority. A new owner preserves commit/tree/ref and creates one draft. | `before-create-*`, paused lookup, real publisher Pod deletion and increased ownership generation. |
| Remote create before response | Interrupt the publisher after the fixture durably creates a draft but before returning its response. Cancellation is rejected; the next owner reconciles by reads without a second POST. | `after-create-*`, paused create response and exact retained draft/commit/tree. |
| Ambiguous create without PR | A consumed permit with no observable PR stays uncertain after the remote recovers and publisher/owners restart. Cancellation cannot erase the ambiguity. | `uncertain-*`; remote counters remain unchanged and ownership generations only advance. |
| Remote identity and conflicts | Response loss recovers the original PR; closed/merged history does not create another draft; a conflicting branch is never overwritten. | Original publication/closure replay and `collision-final.json`. |
| Retained owner restart | Control plane, publisher, fixture, supervisor, executor, broker and PostgreSQL individually restart while original draft, cancellation and uncertain evidence remain queryable. | `restart-*`, `replay-*`, `before-create-replay-*`, `after-create-replay-*` and `uncertain-replay-*`. |
| Cleanup and redeployment | All owned Pods stop; parent/worker PVC UIDs and volume bindings and application Secret identities remain unchanged. Reinstall replays approval, preparation, coding, quality and publication without new external actions. | `cleanup-*` inventories and canonical exact-retry proofs. Helm release bookkeeping Secrets are excluded from the application-identity comparison. |

The final remote fixture retains three PRs from four create requests: the original
draft, the two interrupted publisher cases and one uncertain request without a PR.
The harness also compares the repository manifest and revision before and after
the run, rejecting a bundle if its source changed during execution. Counts remain
unchanged through owner restarts and cleanup/redeployment. Source,
quality and publication evidence remain separate throughout.

## Required repository gates

Run [CONTRIBUTING.md](../../../CONTRIBUTING.md), including
`scripts/ci/check-workflow-m1.sh`, alongside the deployed profile. Real PostgreSQL
suites cover transactional rollback, scope isolation, competing keys, expired
leases/deadlines, stale owners and consumed sending permits. Worker tests cover
provider request uncertainty, frozen source, hostile evidence and sandbox limits.
These complement deployed interruption tests; neither suite substitutes for the
other.

## Evidence limits and operator recovery

Coding uses the explicitly enabled deterministic fixture. Quality uses actual
managed Ayni 0.16.2 on the worker topology; its unsigned local certificate proves
runtime consistency. Git transport is real, while GitHub responses and retained PR
history come from the local protocol fixture. The profile establishes neither
paid coding-provider behavior nor live GitHub authentication, remote signing
policy compatibility or cluster NetworkPolicy enforcement.

To inspect a failure, retain the namespace and evidence directory. Use the Tasks
view or scoped reads for attempt, patch, quality and publication identities.
Replay ambiguous commands with their original content and key. A consumed create
permit permits reconciliation reads, never another create request. An uncertain
coding call is not retried, and deleting its Pod or ledger cannot authorize a new
paid call.

Execution and quality problems also retain conversation feedback and project
attention under the [task feedback contract](../architecture/task-feedback.md).
The same failure/restart cases must preserve exact notice identities and origin,
with duplicate reports and stale workers unable to create duplicate feedback.
The delivery PR owns executed feedback evidence. Restart equality compares immutable notice
facts separately from the live linked-attempt state.

Interrupted coding and failed quality require an exact reviewed recovery proposal with
explicit bounded consent, or a new reviewed plan when approved inputs materially change. Dependency scheduling and concurrent
execution remain M5. Users retain merge authority; successful M4 acceptance ends
at a reviewable draft.

## Recovery review extension

The cumulative coding and quality profiles also record a recovery proposal with frozen
saved-work evidence and additional allowance, an operator question and its first answer
when the attempt has feedback. They verify exact retries, reject another answer and a
foreign-origin read, and preserve the attempt unchanged. Stored recovery bytes are
compared after the existing control-plane and PostgreSQL restart gates. This proves
review-record durability, not confirmed continuation or live provider recovery.
See [recovery review](../architecture/task-recovery.md).

The `--recovery` browser profile requires an already completed fixture attempt with
actionable feedback and no consumed recovery decision. It checks saved-work review,
explicit allowances, accepted response loss, exact retry after reload, operator
questions/answers, retained attention and unchanged task execution. Use the fresh
fixture profile below for a reproducible complete conversation scenario.

## Confirmed continuation extension

The coding and quality profiles confirm one exact recovery proposal concurrently with
different keys, assert one linked attempt, and wait for the real supervisor/worker result.
They compare frozen inputs, saved input patch, independent parent evidence, child result
and exact confirmation replay after the existing control-plane and PostgreSQL restarts.
The failed-quality fixture proposes one bounded repair pass; it does not establish live
provider repair. Successful tasks without feedback create no recovery.

## Complete conversation recovery extension

The `--quality-only` cumulative profile ends with a fresh real managed-quality failure
whose recovery decision is left to the browser. The gateway wrapper compiles the local
acceptance fixture client and console, connects to the deployed authority and records
a separate selection under its evidence directory. Chromium must be installed.

To run that browser gate separately against a ready acceptance namespace and the
local managed fixture runtime:

```sh
AYNI_ORCHESTRATOR_NAMESPACE=ayni-orchestrator-acceptance-continuation \
  dev/scripts/task-console-acceptance.sh /absolute/quality/task-quality-failed-run.json /absolute/new-browser-evidence --fresh-continuation
```

The profile covers two-tab answer conflicts and confirmation convergence, canonical
answer display, explicit consent, accepted proposal/confirmation response loss, exact
retry after reload and navigation to a different conversation while confirmation is
pending. The retry preserves the original task/proposal and cannot render that task
in the newly selected conversation. Parent evidence remains unchanged; conversation
history links the resolved parent decision to the continued coding and quality outcome.
The continued fixture intentionally fails quality again and creates its own open notice.
Screenshots cover desktop and mobile. No live provider or GitHub request is exercised.

PostgreSQL and console source tests additionally prove question-only pages include
answers from later pages, every page carries parent confirmation, alternate proposals
lose consent controls, and unchanged refreshes preserve the loaded review prefix.
M5 dependencies, structured worker questions and cumulative M6 closure remain outside
these gates.
