# MVP delivery milestones

Status: agreed delivery sequence. Each milestone has one delivery PR and an
observable completion gate. M0 is complete; M1 and M2 have executable work and
recorded local deployment evidence but remain incomplete until their gates pass.

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
| M1: managed execution | `feat(worker): execute managed Ayni checks with retained evidence` | A pinned repository revision produces a real managed result | In progress |
| M2: durable orchestration | `feat(control-plane): reconcile durable execution attempts` | Requests survive controller restart and worker failure without duplicate effects | In progress |
| M3: request to draft PR | `feat(tasks): deliver agent changes with validated draft PRs` | One real maintenance request produces a reviewable, independently validated change | Planned |
| M4: pilot-ready product | `feat(mvp): complete repository readiness and task experience` | A maintainer operates the full workflow through its UI and documented setup | Planned |
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

**Selected initial inputs and topology (2026-09-17):** Ayni `0.12.1` is the
pinned worker executable. The only supported source is the public Ayni Rust
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

**Completion gate:** deploy the worker and demonstrate managed success, an
intentional quality failure, stale-lock rejection, and missing or mismatched
artifact rejection. Reproduce a result in a fresh validation workspace. Record
all preparation, image, storage, and engine requirements. If the managed engine
cannot run correctly on the selected local topology, the milestone remains blocked.

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

**Remaining M2 evidence:** Controller-driven managed success, intentional
quality failure, and stale-lock rejection are retained. The required missing-or-
mismatched-artifact rejection and a fresh validation-workspace reproduction
still need managed evidence. Complete those M1 gates before marking either
milestone complete.

**Completion gate:** deploy the orchestration slice and exercise controller
restart, duplicate events/reports, worker loss, cancellation, late completion,
and cleanup alongside another active attempt. Task history and retained evidence
survive replacement. PR-creation recovery is exercised in M3 when that action exists.

## M3: request to draft PR

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

**Completion gate:** deploy the final candidate and have a maintainer operate the
full workflow using its interface and documentation. Exercise cold and warm
preparation, a failed replacement environment, and missing/revoked access. Every
applicable [MVP engineering gate](mvp-validation.md#engineering-gates) has passing
evidence on the selected topology. Verify a clean install and an update from the
previous milestone without losing retained task data.

M4 establishes a pilot-ready MVP. It does not establish product value.

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
| M4 completion | Supported dependency-change behavior and all operational limits |
| M5 | Ten tasks, comparison protocol, acceptance definitions, and acceptable cost ceiling |

Docker Desktop Kubernetes and Kustomize are selected. Record the other choices
and their rationale when made, without implying unsupported capabilities or
changes to the independent Ayni product.
