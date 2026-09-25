# MVP delivery milestones

Status: agreed delivery sequence. Each milestone has one delivery PR and an
observable completion gate. M0 through M3 are delivered; M4's local completion
gates pass and its delivery PR is pending branch publication.

Read the [MVP scope](mvp.md) for the product boundary and the
[validation plan](mvp-validation.md) for engineering gates and pilot evaluation.

## Delivery rules

- Deliver M0 through M5 in order, with one focused PR per milestone. Planning
  documentation is separate from those delivery PRs. The table's PR titles are
  intended titles, not claims that PRs already exist.
- After the preceding PR merges, update local main and create a fresh descriptive,
  unbranded branch. Keep implementation, checks, deployment evidence, and relevant
  documentation together in that milestone's PR. Do not merge automatically.
- Deploy to Docker Desktop Kubernetes as soon as the milestone has executable
  behavior. M0 includes the first running service. Exercise later behavior in the
  cluster during development and validate the final candidate before completing
  the PR; do not defer deployment until the full MVP is built.
- Use the selected [Kustomize deployment approach](decisions/0001-local-kubernetes-deployment.md).
  Each runtime change must preserve the local build/deploy/check workflow.
- Record the tested commit and actual image identity, cluster/tool versions,
  commands, results, and limitations in each PR. If the source changes afterward,
  rebuild and rerun affected deployment checks.
- Commit and push completed work even when infrastructure blocks a required check,
  but mark the PR and milestone blocked or incomplete. Mocked or host-only results
  cannot substitute for required managed execution evidence.
- Update this document's status and PR reference as each milestone progresses.
  Change the plan explicitly if new evidence requires different scope.

## Sequence

| Milestone | Intended delivery PR title | Observable result | Status |
| --- | --- | --- | --- |
| M0: foundation and first deployment | `chore(workspace): establish Rust workspace and local deployment` | A checkout-built control-plane service runs on Docker Desktop Kubernetes | Complete — evidence predates this repository import |
| M1: managed execution | `feat(worker): execute managed Ayni checks with retained evidence` | A pinned repository revision produces a real managed result | Complete |
| M2: durable orchestration | `feat(control-plane): reconcile durable execution attempts` | Requests survive controller restart and worker failure without duplicate effects | Complete |
| M3: request to draft PR | `feat(tasks): deliver agent changes with validated draft PRs` | One real maintenance request produces a reviewable, independently validated change | Complete |
| M4: pilot-ready product | `feat(mvp): complete repository readiness and task experience` | A maintainer operates the full workflow through its UI and documented setup | Complete locally — delivery PR pending |
| M5: product validation | `docs(pilot): report MVP outcomes and investment decision` | Ten real tasks support an advance, iterate, or reconsider decision | Planned |

## M0: foundation and first deployment

Build the private Rust workspace with `domain`, `control-plane`, and `worker`.
Keep application dependencies directed toward `domain`. Pin the Rust toolchain,
commit the dependency lock, add the complete pinned rust-skills source and rules
with attribution, and establish CI and the repository's Ayni quality contract.

The control-plane executable supplies validated configuration, structured
diagnostics, liveness/readiness endpoints, and graceful shutdown. Readiness
describes only functionality present at this milestone; it must not imply that
workers or task execution are ready. The worker crate need not pretend to
implement execution yet.

Add the control-plane image definition, Kustomize base and Docker Desktop overlay,
and a repeatable local build/deploy/check workflow. Establish image delivery to
the actual Kubernetes runtime rather than assuming a Docker build automatically
makes an image available to its nodes.

**Completion gate:**

- A clean checkout passes formatting, linting, tests, compile checks, and its
  declared quality policy with the pinned toolchain.
- Docker Desktop Kubernetes is configured and node readiness is checked through
  the explicit `docker-desktop` context.
- The checkout-built service deploys, exposes health endpoints through loopback
  access, and terminates cleanly when replaced.
- Reapplying the deployment is safe; a rebuilt candidate reaches the running pod.
  Installation and scoped cleanup are documented and exercised.
- The PR records image identity and deployment evidence, with no claim that the
  managed worker or full MVP is complete.

## M1: managed execution

Choose one supported repository shape and pin an Ayni executable/executor. Add
the bounded worker path: fetch an exact revision, validate declared inputs,
prepare its managed environment, run an unscoped Ayni check, and retain original
evidence with source/environment identity. Keep failure classes distinguishable.

Run the worker through a manually submitted local Kubernetes Job initially.
Validate the engine/worker arrangement described in the
[deployment decision](decisions/0001-local-kubernetes-deployment.md#managed-worker-boundary).
A standalone Linux experiment may inform that choice, but does not replace the
Docker Desktop deployment gate.

**Selected initial inputs and topology (2026-09-17; historical, superseded by
the 2026-09-24 migration):** Ayni `0.12.1` was the pinned worker executable.
The initially supported source was the public Ayni Rust
workspace at release commit `42f2605d14aa980d14608a8d2f518bc05fcc47b6`, with
`.ayni.lock` SHA-256
`606972cd98e464c8a8befc5191b770539637a9ef3b872ea3ec52abae92928e4f`.
The manual Job runs the worker beside a digest-pinned Docker-in-Docker engine;
both share attempt-local workspace storage, while only the worker mounts the
retained-evidence PVC. The engine is privileged but is not shared with the
control plane, host Docker socket, or a repository-controlled process.

**Deployment evidence (2026-09-17, commit `64cf81f`):** local Rust and
Kustomize validation passed. Docker Desktop exposed the required
`docker-desktop` context but its Kubernetes API returned EOF during readiness
and submission checks; Docker Desktop's engine did not return a usable
control-plane status. No M1 Job or evidence PVC was created. Managed execution
evidence is therefore pending local cluster recovery.

**Additional deployment evidence (2026-09-17, commit `3221d0e`):** A fresh
manual Docker Desktop Job completed the pinned managed check successfully using
`local/ayni-orchestrator-worker:git-3221d0e`. The isolated Docker-in-Docker
engine and non-root worker both exited `0` after approximately four minutes.
The worker supplied its legacy-builder architecture explicitly, included the
pinned Buildx plugin, kept Docker client state in an attempt-local scratch
directory, and placed Ayni temporary bind inputs on the shared attempt
workspace. Its retained manifest for attempt
`4d329019-701b-4007-aaeb-c94bda889b09` records outcome `success`, check exit
code `0`, the selected source and lock identities, and SHA-256 digests for
`signals.json`, `execution.json`, and `build.json`. The retained evidence PVC
remained `Bound` after read-only inspection.

**Additional deployment evidence (2026-09-17, commit `3f6ab13`):** A separate
manual Job supplied an all-zero expected lock digest and was rejected after the
pinned source had been fetched: the worker recorded the expected and actual
digests and exited nonzero. Its retained evidence directory contains the source
preparation command logs; only that completed Job and its unique ConfigMap were
deleted, leaving the shared evidence PVC `Bound`. A second test-only Job changed
one assertion in the attempt-local workspace after the allowlisted source and
lock had been validated. The worker completed normal managed preparation and
retained an Ayni manifest with `outcome: quality_failure`, check exit code `4`,
and SHA-256 digests for `signals.json`, `execution.json`, and `build.json`.
The test Job and temporary sidecar were removed only after that manifest was
read through a temporary read-only inspector; retained evidence was preserved.

**Final completion evidence (2026-09-20, commit `9ec53c5`):** A clean detached
validation worktree at `9ec53c5e3fe0b40610f23dec2f77bce10aaf276e` rebuilt and
loaded `local/ayni-orchestrator-worker:git-9ec53c5e3fe0` (image index
`sha256:5f7b522606728acca37efd7433f82a13c77a5ad77cb48842fb24a632f989d9c4`)
into Docker Desktop Kubernetes `v1.36.1`. The dedicated, separately named
artifact-rejection Job ran the real managed check through the normal isolated
engine, then a test-only mounted wrapper removed `execution.json` before the
unchanged worker's artifact-binding step. After 4m51s the worker failed with
`missing execution artifact: /workspace/repository/.ayni/last/execution.json`;
the Job did not complete and `managed-check-evidence` remained `Bound`. The
wrapper exists only in the dedicated test manifest and does not alter the worker
image or normal managed-check Job. This provides the required missing-artifact
rejection and fresh-workspace reproduction evidence.

**Completion gate: complete.** The worker was deployed with managed success, an
intentional quality failure, stale-lock rejection, and missing or mismatched
artifact rejection. A result was reproduced from a fresh validation workspace;
the preparation, image, storage, and engine requirements are recorded above.

## M2: durable orchestration

Choose the durable store, artifact store, worker transport, and initial Kubernetes
resource model. Assign one owner for each state transition. Persist tasks,
attempts, desired inputs, checkpoints, and external-action identities outside
disposable workers. A minimal API submits, inspects, cancels, and retries work.

The controller schedules real workers, records observations, applies bounded
retry and execution limits, and cleans up only owned resources. Introduce
generated custom-resource definitions when the selected design needs them.

**Selected initial design (2026-09-17):** the single-replica local control plane
uses SQLite on a retained PVC for tasks, attempts, idempotency/external-action
records, and worker reports. Managed evidence remains on the retained
`managed-check-evidence` PVC and is referenced by the immutable attempt identity.
The in-cluster control-plane service account creates, reads, and deletes only
deterministic attempt-owned Jobs in its namespace. A reconciliation loop creates
an absent queued Job, records observed Job state, requests cancellation, and marks
an absent non-terminal Job interrupted. It never automatically retries an attempt;
an explicit, idempotent retry creates the next attempt. Details and the
single-replica/PVC constraint are in [Decision 0003](decisions/0003-durable-local-orchestration.md).

**Deployment evidence (2026-09-17, commit `65447f0`):** Docker Desktop
Kubernetes `v1.36.1` on the explicit `docker-desktop` context accepted the
rendered deployment, bound both retained PVCs, and ran the control plane using
`local/ayni-orchestrator-control-plane:git-65447f06ad23`. Its `/livez` and
`/readyz` endpoints passed; readiness reported durable orchestration at domain
model revision 2. The worker image
`local/ayni-orchestrator-worker:git-65447f06ad23` was loaded into the node and
recorded with each attempt. An idempotent submission created one Job; replacement
of the control-plane Deployment retained the task and found that same Job rather
than creating another. Cancellation removed only the owned Job and a late success
report left the attempt `cancelled`. A separately submitted Job deleted to model
worker loss became `interrupted` with its retained identity. Local formatting,
Clippy, tests, and Kustomize rendering passed.

**Additional deployment evidence (2026-09-17, commit `bf4ebaa`):** The
controller Deployment was rebuilt from
`local/ayni-orchestrator-control-plane:git-bf4ebaa` and upgraded its retained
SQLite database in place on Docker Desktop Kubernetes `v1.36.1` (kubectl
`v1.35.2`, Kustomize `v5.7.1`). A task persisted its `max_attempts: 3` bound:
three failed attempts remained in history and a fourth retry returned `400` with
`task has reached its maximum of 3 attempts`. Two deliveries of the same worker
report key both returned `204` without adding another state transition; the
store test also verifies that only one report record is retained. For cleanup,
controller-created Job `managed-check-01a0b07d5c63759e830a` was given a
test-only 30-second deadline and TTL. Kubernetes removed that terminal owned Job
while independent controller-created Job
`managed-check-01a0b07d5c7d74c0bde8` was still active. Both retained PVCs stayed
`Bound`. The peer later reached its normal terminal deadline and was left for
its configured one-hour Kubernetes TTL; neither retained PVC was deleted.

**Additional deployment evidence (2026-09-17, commit `3f6ab13`):** Docker
Desktop Kubernetes `v1.36.1` ran the controller from
`local/ayni-orchestrator-control-plane:git-3f6ab13`, with its matching revision
label and image identity, and used
`local/ayni-orchestrator-worker:git-3f6ab13` for a new managed attempt. Task
`01a0b12f-28b9-748b-821b-c71f49115da7` created deterministic Job
`managed-check-01a0b12f28b9748b821b`; it completed the pinned Ayni check
successfully. The durable task record then reported its first attempt
`succeeded`, retaining the Job name, worker image, source revision, lock digest,
and the configured maximum of three attempts. A read-only inspection of attempt
`01a0b12f-28b9-748b-821b-c7207bf3a759` confirmed a success manifest with exit
code `0` and SHA-256 digests for `signals.json`, `execution.json`, and
`build.json`; the retained evidence PVC remained `Bound` and the temporary
inspector pod was deleted.

**Final M2 completion evidence (2026-09-20, commit `9ec53c5`):** M1's remaining
missing-artifact rejection and fresh-workspace reproduction gates passed in the
same selected Docker Desktop topology; see M1's final completion evidence above.
Together with the controller-driven success, quality failure, stale-lock
rejection, restart, duplicate-report, worker-loss, cancellation, late-report,
retry-limit, and cleanup evidence recorded above, this completes M2.

**Completion gate: complete.** The orchestration slice was deployed and exercised
for controller restart, duplicate events/reports, worker loss, cancellation, late
completion, and cleanup alongside another active attempt. Task history and
retained evidence survived replacement. PR-creation recovery remains M3 work.

## M3: request to draft PR

**Selected M3 design (updated 2026-09-24):** Only the private
`ayni-oss/sandbox` Rust workspace is accepted for live M3 tasks. Its generated
`.ayni.lock` from pinned Ayni 0.13.0 binds the environment. The sandbox is
initialized with the M3 fixture and workspace contract before testing, keeping
deliberately broken formulas, generated branches, and draft PRs out of the
control-plane repository. An OpenAI-backed Rig coding Job receives fixed file and
command tools in an attempt-owned checkout, then writes a local commit and Git
bundle to a retained checkpoint. A separate Job checks out that frozen bundle,
runs the unscoped managed Ayni check, and retains hash-bound evidence outside
the coding workspace. Only after success does a separate Rig PR-author Job call
the control plane's structured, idempotent draft-PR action. The control plane
holds the GitHub App key, mints repository-scoped installation tokens, and
reconciles the deterministic task branch and PR. Other agent providers can be
configured but receive an explicit unsupported result. The live proof uses the
simple retry-backoff fixture, with expected seconds `1→2`, `2→4`, `3→8`, and
`6→60`; its starting formula is deliberately incorrect.

**Validation status:** Local Rust formatting, Clippy, tests, compile checks,
Kustomize rendering, generated-lock verification, Docker Desktop deployment,
and live API-to-draft-PR evidence must all be recorded against the final
committed revision. A local test or simulated GitHub response is not a
substitute for the deployed live proof. The narrow GitHub App installation and
local Kubernetes Secrets are prerequisites for that final exercise. The draft
PR targets the sandbox's `main` branch; neither draft PR nor task branch is
merged automatically.

**Local validation evidence (2026-09-24, code commit `6dc4210`):** Rust formatting,
Clippy, workspace tests (23 passing), and compile checks passed. The unscoped
managed Ayni 0.12.1 check passed all four signals (tests, size, complexity,
dependencies) using the generated lock SHA-256
`b450f91e0b4f1d1733ab47cfa256a5ad59fb273ac615039d30da22b1d46f44e9`;
`env build` and `env doctor` succeeded. The M3 Kustomize overlay rendered.
The worker image built and a read-only-root smoke test found Ayni 0.12.1,
Rust/Cargo 1.94.0, Git 2.39.5, and all three M3 executables.

**Ayni source migration (2026-09-24):** The active worker binary download,
managed-check source fixture, and generated orchestrator lock now target the
supported [`ayni-oss/ayni` 0.13.0 release](https://github.com/ayni-oss/ayni/releases/tag/ayni-v0.13.0).
The source fixture pins release commit
`4e67795c0cba21bd82c21bdd6c921417fbe29ff6` and its `.ayni.lock` SHA-256
`d68ca2ac621c99865bf7b50aabc668f1e9e4b85aa79389c00e13fcda4bb56fc7`.
The orchestrator's regenerated lock SHA-256 is
`7e324054c9f5663ab361ef7d168dbe1a7813fecea1072f0ec9f2cdaf7b901515`.
The earlier 0.12.1 validation above remains historical evidence, not validation
of this migration. Ayni 0.13.0's lock pins
`ghcr.io/ayni-oss/ayni-provisioning@sha256:30453fcd751acff069c7b16a1cd2eba01051bd94b64cd9d333c4ce82a9ff0961`,
which GHCR currently reports as `not found`; managed environment build and the
live M3 proof therefore remain blocked pending a published supported image.
For this migration, `cargo fmt --check`, Clippy, workspace tests (23 passing),
`cargo check`, the M3 Kustomize render, and Ayni 0.13.0's unscoped host check
(4/4 signals) passed. A managed `env build` with the published, digest-pinned
`ayni-oss/ayni-env:0.13.0-debian` executor failed resolving that provisioning
image; this is not a passing managed check or deployment result.

**Blocked Docker Desktop deployment (2026-09-24, code commit `6dc4210`):**
`scripts/local/deploy.sh m2` built and loaded
`local/ayni-orchestrator-control-plane:git-6dc421069608` (image index
`sha256:fd755e3e13ac59756046f9cf69302ea9cfffa21d5750c8bbd95268da725f1a8f`)
and `local/ayni-orchestrator-worker:git-6dc421069608` (image index
`sha256:2db0c6fe418ad418ddd7d97c77bc2bcff5a5f292ad5c926772cb909581b7b45c`).
The pod reported control-plane image ID
`sha256:9c6332c90e6be574c69c43d91995bf1d84b08522c57ad091b09cb52facc0e3e3`,
but SQLite startup failed with `database or disk is full`. Docker Desktop's
Kubernetes node filesystem reported 59 GiB total, 0 available, 100% used;
the fresh namespace had no prior task state to preserve. Docker Engine was
29.8.0, Kubernetes 1.36.1, kubectl 1.35.2, and Kustomize 5.7.1. No Docker
cache, images, or volumes were pruned. The M3 overlay was not deployed because
the App/OpenAI Secrets and live test were explicitly deferred. State-survival,
negative-path, and GitHub reconciliation exercises remained unverified at that
revision. This was a historical blocked attempt and is superseded by the final
evidence below.

**Final M3 completion evidence (2026-09-25, commit `617dfd8`):** The previously
missing provisioning manifest was copied without alteration to the public
`ghcr.io/ayni-oss/ayni-provisioning` package at digest
`sha256:30453fcd751acff069c7b16a1cd2eba01051bd94b64cd9d333c4ce82a9ff0961`;
anonymous manifest requests passed for linux/amd64 and linux/arm64. The executor
at `sha256:e21ccf9e66e7490953d2a30ba49ce8894025fd0e59556bc65a3c17c783ae5fe9`
was also anonymously readable, and Ayni's public consumer workflow passed on
both architectures.

The final candidate passed formatting, Clippy, all 25 workspace tests, compile
checks, M3 rendering, server-side manifest validation, and `git diff --check`.
Its regenerated Ayni 0.13.0 lock has fingerprint
`sha256:22f46dceb89c6597d02856c31fed1b627c0dc2b8365675f6e283b58071ee0d43`;
managed `env build`, `env doctor`, and all four Ayni signals passed. Docker
Engine 29.8.0 and Docker Desktop Kubernetes v1.36.1 (kubectl
v1.36.1, Kustomize v5.8.1) ran control-plane image index
`sha256:d4c0a4d27e7da864677e9aff536ad4f294853c02aa99685bf9cf2c5f24bdbb97`
and worker image index
`sha256:ee693ecc0f3df5ba22264768265d2211f3176129c8965c2201072a55c768cb6f`.
The active control-plane container reported image ID
`sha256:21b717828a632fcdcf277610c8b1f867a15023f39a3c986fb976bf634cb68228`,
and readiness passed.

Live task `01a0d75c-5766-7509-b04d-ae856bdb56f0` started from sandbox revision
`589fb2a2fae67d87e00da1ddc4c9a576a207d2ff` with lock SHA-256
`f91be39b21d1af3506e25caf246c0d7178c339669ed511582547acacec8b54d4`.
The coding Job produced candidate
`b8badfdad94249e968c53fc72ed78fd967701282`; independent validation retained
manifest
`sha256:b18d1a228da3a23cfd96535955e25a9e44ac00aaf03cb6063d484ac992e734f4`,
reported no policy or test edits, and passed cases `1→2`, `2→4`, `3→8`, and
`6→60`. The structured publisher created exactly one App-authored draft PR,
[`ayni-oss/sandbox#1`](https://github.com/ayni-oss/sandbox/pull/1), targeting
`main`; its sole changed path is `src/main.rs`, and it remains unmerged for
maintainer review.

The first publication attempt durably recorded a pending action but could not
complete while the read-only control-plane root lacked writable Git scratch
space. After adding a bounded `/tmp` `emptyDir` and replacing the Deployment,
the reconciler recovered that same action and deterministic branch, completed
the task, and produced the single draft PR without a duplicate. Earlier live
task `01a0d4d8-1dc8-7694-acb1-b6f38f43316c` retained a quality-failure manifest
(`sha256:0df39be36a89c9a6a4da58b3989ad15e47d171d18c58c95bd21d09fe2623b9c5`)
and did not publish. Store and API tests additionally cover cancellation,
stale-candidate rejection, immutable task snapshots, one-candidate publication,
and duplicate publication reuse.

**Completion gate: complete.** A real bounded request completed from API
submission through independently measured evidence to one reviewable draft PR.
Quality failure, stale-candidate rejection, cancellation semantics, edit
disclosure, and recovery of an uncertain publication action are covered by the
retained live evidence and final test suite. The draft remains for the
maintainer's merge decision.

Select one agent integration and the GitHub access mechanism. Provide an
attempt-owned development checkout, scoped permissions, repository instructions,
focused Ayni verification, and useful source checkpoints. A request can finish,
fail, or explicitly require input.

Freeze candidate inputs and run final validation separately from the editable
agent environment. Publish one draft PR describing the candidate and evidence.
Final validation and publication must not trust agent-written success claims.
Keep evidence associated with the exact measured candidate.

**Completion gate:** a real bounded maintenance request completes from API
submission to a reviewable draft PR on the deployed local system. Demonstrate
quality failure, changed-candidate invalidation, disclosure of policy/test edits,
and recovery after an uncertain PR creation response without a duplicate PR.
The maintainer decides whether to merge.

## M4: pilot-ready product

Complete the simple task UI, repository enrollment, readiness diagnostics,
compatible preparation reuse, result/evidence views, cancellation, retry, and the
path to provide requested input. Document access, installation, resources,
updates, retention, recovery, and cleanup. Declare whether dependency evolution
supports rebuilding or returns an explicit unsupported outcome.

**Selected M4 implementation (2026-09-25):** the control plane serves an
unauthenticated in-cluster browser interface and durable list/action APIs,
operationally restricted to loopback port-forwarding for the trusted pilot.
Repository preparation retains a digest-bound Ayni image archive and build
record; matching inputs reuse it and final validation loads it into a fresh
isolated engine. Retries and requested input create linked tasks. Rust dependency
evolution is explicitly `dependency_change_unsupported`. Decision
[0004](decisions/0004-pilot-readiness-and-preparation.md) records identities,
security boundaries, and operating limits; the [pilot runbook](pilot-operations.md)
records installation and operations.

**Final deployment evidence (2026-09-25, signed runtime commit
`e17e932fd1c21c43fec1912b84197c4c95c1fc56`):** formatting, Clippy, 33 workspace
tests, compile checks, shell syntax, M4 Kustomize rendering, client dry-run,
isolated clean installation, and the live M4 check passed. Docker Engine `29.8.0`
and Docker Desktop Kubernetes client/server `v1.36.1` with Kustomize `v5.8.1`
ran `local/ayni-orchestrator-control-plane:git-e17e932fd1c2` (OCI index
`sha256:85cf0a9b791fe1e1e1b91a9c6cb1bc55322b2f538c9a916834632753b6edf774`,
running image
`sha256:bee47efb009b298efcbf183a737cdfc21546809dd0fb4d1118535a4838709f93`)
and loaded `local/ayni-orchestrator-worker:git-e17e932fd1c2` (OCI index
`sha256:67f883d92d967b76a195c1652857ef5d6f0624f4610d6efe5933b2bf62285cb6`).
The rollout retained all four PVCs and prior tasks. `pilot_ready: true` records
publisher/control-plane configuration; each repository has its own preparation
readiness state.

Repository `ayni-oss/sandbox` revision
`589fb2a2fae67d87e00da1ddc4c9a576a207d2ff` cold-prepared in 106 seconds as
`prep-c536767dd800a652ba3f8e7a8c23aed7bda3e4c11e1edeb73b94fda063359d02`.
Repeated API and browser preparation actions reused that identity and raised
`reuse_count` to 3. A changed-lock replacement,
`prep-085ef6186f424e17d0efdcf9dd58b5cb110df6addbc769487b104c443709ce77`,
failed, and a task for those desired inputs returned HTTP 400 rather than using
the older preparation. Restoring the original lock warm-reused the original
identity.

The browser submitted task `01a0d8a1-ef61-7124-ba0a-9f50d1578640`, which
completed from prepared revision to candidate
`6372f45a8da727db395893a541e65374db6dbb01`, retained manifest
`sha256:487da11b516e05576ba71f86b1323533b9fbd415a158b08c6ecb3addfc953a71`,
and exactly one draft [`ayni-oss/sandbox#3`](https://github.com/ayni-oss/sandbox/pull/3).
Chrome displayed its success evidence through the rendered **View** action. The
browser also submitted warm preparation, provided input to waiting task
`01a0d884-27b9-7511-a653-af10582edf6d`, created a linked follow-up, and cancelled
it through the rendered action.

The [M4 engineering-gate record](m4-engineering-gates.md) maps every required
scenario to its expected result and deployed evidence. It includes current-path
artifact alteration/removal, immutable post-validation candidate binding, inline
test and policy-edit disclosure, dependency rejection, controller restart
without a duplicate Job, validation-worker loss with a retained candidate,
cancellation, access loss after readiness, failed replacement, and requested
input. A reproducible namespace override now supports isolated clean-install
checks. The in-place M3-to-M4 update and final rollout retained prior task data and
PVC identities.

**Completion gate: complete locally; delivery PR pending branch publication.**
The deployed candidate and documented browser workflow satisfy the M4 engineering
and operating gates. M4 establishes a pilot-ready MVP; it does not establish
product value or authorize M5 conclusions.

## M5: product validation

Select the ten real tasks, acceptance criteria, baseline protocol, operating
budgets, and cost ceiling before collecting results. Use the deployed M4 candidate
and record its exact identities. Apply the [pilot protocol](mvp-validation.md#ten-task-pilot),
including human review, comparison limitations, failure accounting, and fresh
validation of claimed verified results.

**Completion gate:** commit a private pilot report with per-task evidence,
aggregate measurements, limitations, and the investment decision. An inconclusive
or negative result is a valid completed experiment when reported accurately.
Missing runs or evidence must remain explicit and cannot establish validation.
Code fixes discovered during the pilot require recorded validation; do not mix
outcomes from unrecorded implementation changes.

The M5 PR records the decision and next work. It does not automatically authorize
multi-repository expansion, public publication, or production deployment.

## Decisions needed before implementation advances

| Before | Required decision |
| --- | --- |
| M0 deployment | Docker Desktop cluster setup/version, local image delivery, and resource allocation |
| M1 | First repository shape, pinned Ayni version, and tested managed worker/engine arrangement |
| M2 | Selected — [Decision 0003](decisions/0003-durable-local-orchestration.md) records local durable storage, retained evidence, Job transport, and lifecycle schema |
| M3 | First agent, GitHub access, and checkpoint/candidate handoff |
| M4 completion | Selected — dependency changes are explicitly unsupported; [Decision 0004](decisions/0004-pilot-readiness-and-preparation.md) records the operating limits |
| M5 | Ten tasks, comparison protocol, acceptance definitions, and acceptable cost ceiling |

Docker Desktop Kubernetes and Kustomize are selected. Record the other choices
and their rationale when made, without implying unsupported capabilities or
changes to the independent Ayni product.
