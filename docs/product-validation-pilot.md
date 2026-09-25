# Product-validation pilot record

Status: ready to run on 2026-09-25. No paid pilot run has started and no
product-value outcome is claimed. The merged sandbox baseline, five independent
baseline failures, exact paired inputs, and both five-minute cancellation paths
are validated. The next action is the first frozen pair, not an additional probe.

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

Every Orchestrator submission carries `deadline_seconds: 300`. The control plane
persists one absolute deadline, gives every stage Job no more than the remaining
time, fails an unfinished task closed at the cutoff, and removes its owned Jobs,
Pods, and clone secret. The manual arm runs the same agent binary in a dedicated
Kubernetes Job with a 300-second active deadline and ten-second termination
grace. No candidate produced after the cutoff is eligible.

The harnesses verify that no owned Job or Pod remains and use forced, label-scoped
deletion only if normal deadline cleanup does not finish. The non-provider cutoff
probes completed before the cohort: Orchestrator task
`01a0d998-4d0c-76a2-8f57-722bee79f39d` failed with `task deadline exceeded`
and no resources remaining; manual Job `m5-gcd-manual-cutoff-v3-agent` reached
`DeadlineExceeded` and had no resources remaining. Primary runs do not retry or
accept follow-up input: failure, timeout, or requested input is the row outcome.
Any diagnostic retry remains associated with the row and cannot reset its durable
deadline or create another primary success opportunity. Record provider charge
when available; a time bound is not a guarantee of zero cost.

## Implementation-readiness baseline

The M5 implementation-readiness candidate is based on pulled `main` revision
`f85a3cae0a7bb297664770b753534ed0d3e0e50d` and deployed from the clean pilot
branch:

- source: `8d01e5e3b45b37813ba707663f03f578e82b2174`
- control-plane image: `local/ayni-orchestrator-control-plane:git-8d01e5e3b45b`
- control-plane OCI index: `sha256:ada65e92847feb61686fad104329f96611930281f5152e42c8d706886c3a6798`
- running image ID: `sha256:237bcbf8d7f8796d85b8853d477a12eb1bf7051840a3ad0cbfc7a249f0286b77`
- worker image: `local/ayni-orchestrator-worker:git-8d01e5e3b45b`
- worker OCI index: `sha256:9b6a91e4bfcb9914937e9d45dfe12679544d1f4de42ea4edf4ee28f16c9afd29`
- topology: Docker `29.8.0`; Kubernetes `v1.36.1`; Kustomize `v5.8.1`;
  `docker-desktop` context and `ayni-orchestrator-dev` namespace
- readiness: managed execution, durable orchestration, task UI, repository
  preparation, and pilot configuration all available
- retained storage: all four expected PVCs `Bound`
- enrolled sandbox revision: `916f0184bab0999c1a7172dce84898137923505c`
- sandbox lock file SHA-256: `c81a8359c237dd1ef0583167617ee5d36c9e83789ca11e510f2b1f80bc7b325a`
- ready cold preparation: `prep-20b20e9229c2491d27e3c07b1283d89e7e72398094f957d6d72c320eba0bc871`
  in 103 seconds

Formatting, Clippy with warnings denied, all-feature workspace tests (37 passing),
compile checks, shell syntax checks, deployment, rollout, local readiness, additive
migration, and both cutoff paths passed. This confirms implementation readiness
only; it does not count as a paid product-validation run.

## Paired-agent equivalence

Both arms must use the `development-agent` binary from the recorded worker image,
the `openai-gpt5` profile, its fixed preamble, the same read/write/command tools,
the same 24-turn limit, and identical request and acceptance-criteria bytes. The
Orchestrator arm uses normal controller scheduling and checkpointing. For the
manual arm, the maintainer prepares an isolated checkout and invokes that same
binary directly; only preparation, coordination, evidence handling, and recovery
are manual. `scripts/pilot/math-cohort.json` is the sole request and criteria
source. `run-orchestrator-math-arm.sh` submits it with the durable deadline;
`run-manual-math-arm.sh` prepares a private PVC subpath and launches the same
worker image with matching 2-CPU/3-GiB limits, Cargo seed, security context,
model, and serialized criteria. Any unavoidable environment, resource, or tool
difference makes the pair non-equivalent and must be reported.

The candidate, transcript, summary, and test output from the first arm remain
hidden until the second arm reaches a terminal state. The maintainer may operate
the second arm but must not inspect the first solution. If information crosses
between arms, mark the pair contaminated and exclude it from comparative claims.

## Repository eligibility review

The enrolled sandbox revision contains the previously exposed retry-backoff
fixture and is not an eligible cohort base. Merged sandbox revision
`916f0184bab0999c1a7172dce84898137923505c` prepares a passing
`math-validation` CLI with lock fingerprint
`sha256:96714feaa1986289c5f1284259c53887e6bfd073510ba0399e4580188b0c1eec`,
repairs that known fixture, and deliberately does not implement or plant failures
for the five selected feature tasks. After rebuilding the pinned Ayni environment,
all five independent validators returned exit 1 with the baseline's retry-delay-only
usage message. These are intended absent-feature failures, so this revision is the
frozen common start for both arms.

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

Operate one arm at a time with `run-orchestrator-math-arm.sh TASK RUN_ID
http://127.0.0.1:18080` or `run-manual-math-arm.sh TASK RUN_ID WORKER_IMAGE`.
Use unique lowercase run IDs, the recorded worker image, and a loopback-only API
port-forward. Before the manual arm, retain the verified baseline Git bundle as
ConfigMap `m5-sandbox-baseline`; its checkout preparation remains outside the run
clock. The launchers emit only phase/timing records and do not reveal candidates.

Complete provider usage and full tool transcripts are not exposed by this build.
Record provider cost as missing unless independently available, and do not infer
zero cost. An Orchestrator-first success creates a visible draft PR; the operator
must not open it before the paired manual arm terminates. Report any accidental
inspection or solution transfer as contamination.

## Cohort registry

A row becomes eligible only when the shared baseline revision is merged and its
independent validator fails before the feature is implemented. Fixed-arity
commands must reject missing, extra, malformed, or out-of-range input with a
nonzero exit. The variadic checked mean must require at least one value and reject
any malformed or out-of-range value.

| # | Task and independently checked acceptance criteria | Start revision | First arm | Eligibility |
| --- | --- | --- | --- | --- |
| 1 | Add `gcd <u64> <u64>`; print `6` for `54 24`, `9` for `0 9`, and `0` for `0 0` | `916f0184bab0999c1a7172dce84898137923505c` | Orchestrator | Eligible |
| 2 | Add checked `lcm <u64> <u64>`; print `42` for `21 6` and `0` for `0 9`; reject overflow | `916f0184bab0999c1a7172dce84898137923505c` | Manual Ayni | Eligible |
| 3 | Add `is-prime <u64>`; print lowercase booleans; classify `0`, `1`, `2`, `97`, and `100` correctly | `916f0184bab0999c1a7172dce84898137923505c` | Orchestrator | Eligible |
| 4 | Add floor `integer-sqrt <u64>`; handle `0`, `15`, `16`, and `u64::MAX` | `916f0184bab0999c1a7172dce84898137923505c` | Manual Ayni | Eligible |
| 5 | Add `checked-mean <i64>...`; require input, accumulate without `i64` overflow, and truncate toward zero | `916f0184bab0999c1a7172dce84898137923505c` | Orchestrator | Eligible |

`scripts/pilot/check-math-task.sh TASK CHECKOUT EXPECTED_COMMIT
EXPECTED_LOCK_FINGERPRINT EXPECTED_START_COMMIT` is the independent executable
acceptance oracle. It rejects a wrong repository, commit, ancestry, lock, dirty
checkout, or unlocked Cargo build.
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
