# Product-validation pilot record

Status: preparation started on 2026-09-25. No pilot run has started and no
product-value outcome is claimed. The five-task synthetic cohort remains blocked
until the clean sandbox start revision and paired baseline and five-minute
cancellation harnesses are validated.

## Agreed protocol

| Decision | Frozen value |
| --- | --- |
| Repository | Private `ayni-oss/sandbox` repository |
| Comparison | Paired independent Orchestrator and manual-Ayni runs |
| Agent profile | `openai-gpt5`: provider `openai`, model `gpt-5`, no per-run overrides, identical for both arms |
| Ayni/runtime | Ayni `0.13.0`; Rust `1.94.0`; current Docker Desktop worker topology |
| Credential policy | Use existing references; never record secret values or credentials in pilot evidence |
| Run order | Alternate which arm runs first for successive tasks |
| Time limit | Five minutes per independent run, including retries |
| Primary runs | Ten maximum: two arms for each of five tasks |
| Monetary limit | None; the maintainer selected a time-only bound, which does not cap paid provider spend |
| Claims allowed | Synthetic workflow evidence only; no product-value or cost-acceptability conclusion |

Repository onboarding and cold or warm preparation happen before the run clock
and are measured separately. For the Orchestrator arm, start a monotonic timer
when the immutable development request is accepted. For the manual arm, start it
when the prepared checkout and identical immutable request are handed to the
baseline harness and the agent process is launched.

Run the baseline arm in a dedicated process group or container so its agent and
all commands it spawns share one termination boundary. At 300 seconds, classify
unfinished work as timed out and initiate cancellation: use the supported task
action for Orchestrator and send `SIGTERM` to the complete baseline process group
or stop its container. No candidate produced after the cutoff is eligible.

Allow at most ten additional seconds for shutdown. Then send `SIGKILL` to the
complete baseline group or remove its container. For Orchestrator, force-delete
all Jobs and Pods owned by that development task if reconciliation has not removed
them. Verify that no owned process, Job, or Pod remains, and record both cutoff and
actual shutdown latency. Orchestrator cancellation is asynchronous, so 300
seconds is the outcome cutoff rather than a claim that compute has already
stopped. Validate both paths before the cohort starts. Retries remain part of the
original run and do not restart its clock. Any provider charge is recorded when
available; a time bound must not be represented as a guarantee of zero cost.

## Implementation-readiness baseline

The final implementation-readiness candidate is based on the latest pulled
`main` revision `f85a3cae0a7bb297664770b753534ed0d3e0e50d`:

- source: `e4464ea4be7e3253a67f78a3f64a49b4b53df1aa`
- control-plane image: `local/ayni-orchestrator-control-plane:git-e4464ea4be7e`
- running image ID: `sha256:f6488f641b72f892421d7b2e44bc07a67a3e538bb8991293df5cc4966d1e29d3`
- worker image: `local/ayni-orchestrator-worker:git-e4464ea4be7e`
- worker OCI index: `sha256:5ad13ed5fb9019aa66ed172b21e29c37a01d76a8771684ce4664fd057c8ce0fc`
- topology: Docker Desktop Kubernetes, `docker-desktop` context,
  `ayni-orchestrator-dev` namespace
- readiness: managed execution, durable orchestration, task UI, repository
  preparation, and pilot configuration all available
- retained storage: all four expected PVCs `Bound`
- enrolled sandbox revision: `589fb2a2fae67d87e00da1ddc4c9a576a207d2ff`
- sandbox lock SHA-256: `f91be39b21d1af3506e25caf246c0d7178c339669ed511582547acacec8b54d4`
- ready preparation: `prep-c536767dd800a652ba3f8e7a8c23aed7bda3e4c11e1edeb73b94fda063359d02`

Formatting, Clippy with warnings denied, all-feature workspace tests (34 passing),
compile checks, shell syntax checks, deployment, rollout, and the local readiness
check passed from this revision. This confirms implementation readiness only; it
does not count as a product-validation run.

## Paired-agent equivalence

Both arms must use the `development-agent` binary from the recorded worker image,
the `openai-gpt5` profile, its fixed preamble, the same read/write/command tools,
the same 24-turn limit, and identical request and acceptance-criteria bytes. The
Orchestrator arm uses normal controller scheduling and checkpointing. For the
manual arm, the maintainer prepares an isolated checkout and invokes that same
binary directly; only preparation, coordination, evidence handling, and recovery
are manual. Any unavoidable difference in environment variables, resources, or
tooling makes the pair non-equivalent and must be reported.

The candidate, transcript, summary, and test output from the first arm remain
hidden until the second arm reaches a terminal state. The maintainer may operate
the second arm but must not inspect the first solution. If information crosses
between arms, mark the pair contaminated and exclude it from comparative claims.

## Repository eligibility review

The enrolled sandbox revision contains the previously exposed retry-backoff
fixture and is not an eligible cohort base. Sandbox candidate
`6f49ce0508029b96eecc8d07ab58400914ca10c5` prepares a passing
`math-validation` CLI with lock fingerprint
`sha256:96714feaa1986289c5f1284259c53887e6bfd073510ba0399e4580188b0c1eec`,
repairs that known fixture, and deliberately does not implement or plant failures
for the five selected feature tasks. Freeze its eventual merged commit as the
common start revision before running either arm.

The selected tasks are synthetic GCD, LCM, primality, integer-square-root, and
checked-mean CLI features. They are useful for a bounded workflow comparison but
are not genuine maintenance needs. Their results must not be represented as
product-value evidence, regardless of success rate.

## Frozen run procedure

For every eligible task:

1. Record one immutable start revision, request, independently evaluable
   acceptance criteria, known failures, and flaky checks.
2. Freeze the agent/model version and settings, Ayni version and execution
   identity, resources, credential policy, and retention settings. Record
   onboarding and cold or warm preparation duration outside the run clock.
3. Create two isolated workspaces. Do not share candidates, transcripts, or
   solution hints between arms.
4. Run the arm shown first in the registry, then the other arm, with the same
   request, model configuration, and five-minute limit.
5. For the manual baseline, use the same agent with Ayni while a maintainer
   performs preparation, coordination, and evidence handling.
6. Record setup, intervention, recovery, review, and rework minutes separately.
   Keep failed, timed-out, cancelled, and missing-measurement runs in the record.
7. Have the maintainer judge each candidate against the original criteria and
   record acceptance separately from merge status.
8. Freshly validate every Orchestrator candidate claimed as verified. Include at
   least one cold reconstruction during the cohort.

## Cohort registry

A row becomes eligible only when the shared baseline revision is merged and its
independent validator fails before the feature is implemented. Fixed-arity
commands must reject missing, extra, malformed, or out-of-range input with a
nonzero exit. The variadic checked mean must require at least one value and reject
any malformed or out-of-range value.

| # | Task and independently checked acceptance criteria | Start revision | First arm | Eligibility |
| --- | --- | --- | --- | --- |
| 1 | Add `gcd <u64> <u64>`; print `6` for `54 24`, `9` for `0 9`, and `0` for `0 0` | `6f49ce0508029b96eecc8d07ab58400914ca10c5` pending merge | Orchestrator | Blocked |
| 2 | Add checked `lcm <u64> <u64>`; print `42` for `21 6` and `0` for `0 9`; reject overflow | `6f49ce0508029b96eecc8d07ab58400914ca10c5` pending merge | Manual Ayni | Blocked |
| 3 | Add `is-prime <u64>`; print lowercase booleans; classify `0`, `1`, `2`, `97`, and `100` correctly | `6f49ce0508029b96eecc8d07ab58400914ca10c5` pending merge | Orchestrator | Blocked |
| 4 | Add floor `integer-sqrt <u64>`; handle `0`, `15`, `16`, and `u64::MAX` | `6f49ce0508029b96eecc8d07ab58400914ca10c5` pending merge | Manual Ayni | Blocked |
| 5 | Add `checked-mean <i64>...`; require input, accumulate without `i64` overflow, and truncate toward zero | `6f49ce0508029b96eecc8d07ab58400914ca10c5` pending merge | Orchestrator | Blocked |

`scripts/pilot/check-math-task.sh TASK CHECKOUT EXPECTED_COMMIT
EXPECTED_LOCK_FINGERPRINT` is the independent executable acceptance oracle. It
rejects a wrong repository, commit, lock, dirty checkout, or unlocked Cargo build.
After the expected Ayni environment is built, it clones the frozen commit into a
host-owned read-only snapshot, keeps Cargo output outside that snapshot, and runs
candidate code only through the credential-free, network-disabled Ayni
environment. Each candidate command has a 60-second in-container timeout and the
complete Ayni invocation has a 90-second process-group deadline. For each row,
retain its failure against the shared baseline and its result against each frozen
candidate. The coding agent has access only to the sandbox request and checkout,
not this private validator.

## Run ledger

Create one row per arm and retain retries beneath that row rather than counting
new success opportunities.

| Task | Arm | Started/ended | Outcome | Human minutes | First useful verification | Provider cost | Candidate/evidence | Review verdict | Reproduction |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| — | — | — | Not started | — | — | — | — | — | — |

## Completion boundary

After all ten primary runs, calculate every rate separately for the two arms with
five-task denominators, report matched-pair differences, and report every missing
measurement and protocol deviation. Never use ten candidate outcomes over a
five-task denominator or collapse the arms into one result. Apply the decision rules in
[MVP validation](mvp-validation.md#decision-after-the-pilot). An inconclusive or
negative result is valid; missing tasks, missing baseline arms, unexplained
reproduction failures, or absent cost evidence must not be converted into a
successful product claim. Because the tasks are synthetic and there is no
monetary ceiling, this test cannot produce an **Advance** decision or validate
product value; its maximum valid outcome is a workflow-level iterate, reconsider,
or inconclusive result.
