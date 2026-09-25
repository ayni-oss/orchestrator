# Product-validation pilot record

Status: completed on 2026-09-25. All ten frozen primary runs and fresh candidate
reviews are recorded below. The decision is **Iterate within the MVP**: useful
changes were produced, but the Orchestrator arm had a lower strict acceptance rate,
the two arms did not share a comparable terminal endpoint, and human effort and
provider cost were not measured. No product-value or cost-acceptability outcome
is claimed, and this synthetic cohort cannot authorize **Advance**.

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
port-forward. Before the manual arm, retain the verified baseline Git bundle as ConfigMap
`pilot-sandbox-baseline`; the completed run evidence uses its original
`m5-sandbox-baseline` name. Checkout preparation remains outside the run clock. The launchers emit only phase/timing records and do not reveal candidates.

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
| 1 | Add `gcd <u64> <u64>`; print `6` for `54 24`, `9` for `0 9`, and `0` for `0 0` | `916f0184bab0999c1a7172dce84898137923505c` | Orchestrator | Complete |
| 2 | Add checked `lcm <u64> <u64>`; print `42` for `21 6` and `0` for `0 9`; reject overflow | `916f0184bab0999c1a7172dce84898137923505c` | Manual Ayni | Complete |
| 3 | Add `is-prime <u64>`; print lowercase booleans; classify `0`, `1`, `2`, `97`, and `100` correctly | `916f0184bab0999c1a7172dce84898137923505c` | Orchestrator | Complete |
| 4 | Add floor `integer-sqrt <u64>`; handle `0`, `15`, `16`, and `u64::MAX` | `916f0184bab0999c1a7172dce84898137923505c` | Manual Ayni | Complete |
| 5 | Add `checked-mean <i64>...`; require input, accumulate without `i64` overflow, and truncate toward zero | `916f0184bab0999c1a7172dce84898137923505c` | Orchestrator | Complete |

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
| GCD | Orchestrator | 17:24:34–17:27:11Z | Completed in 157s | Not measured | Stage time unavailable | Missing | `O-GCD` | Accepted | Passed |
| GCD | Manual Ayni | 17:27:22–17:28:08Z | Candidate in 46s | Not measured | Not timed | Missing | `M-GCD` | Rejected: prohibited tests | Scope rejected |
| LCM | Manual Ayni | 17:29:09–17:30:07Z | Candidate in 58s | Not measured | Not timed | Missing | `M-LCM` | Accepted | Passed |
| LCM | Orchestrator | 17:30:23–17:32:23Z | Failed in 120s: quality | Not measured | Stage time unavailable | Missing | `O-LCM` | Rejected: tests and quality | Scope and quality rejected |
| Primality | Orchestrator | 17:42:43–17:45:29Z | Completed in 166s | Not measured | Stage time unavailable | Missing | `O-PRIME` | Rejected: prohibited tests | Scope rejected |
| Primality | Manual Ayni | 17:45:39–17:47:31Z | Candidate in 112s | Not measured | Not timed | Missing | `M-PRIME` | Accepted | Passed |
| Integer sqrt | Manual Ayni | 17:48:13–17:49:29Z | Candidate in 76s | Not measured | Not timed | Missing | `M-SQRT` | Rejected: prohibited tests | Scope rejected |
| Integer sqrt | Orchestrator | 17:49:38–17:52:07Z | Completed in 149s | Not measured | Stage time unavailable | Missing | `O-SQRT` | Rejected: prohibited tests | Scope rejected |
| Checked mean | Orchestrator | 17:52:50–17:55:06Z | Completed in 136s | Not measured | Stage time unavailable | Missing | `O-MEAN` | Accepted | Passed |
| Checked mean | Manual Ayni | 17:55:15–17:56:59Z | Candidate in 104s | Not measured | Not timed | Missing | `M-MEAN` | Accepted | Passed |

### Candidate and evidence registry

| Key | Task/candidate evidence |
| --- | --- |
| `O-GCD` | Task `01a0d998-f363-7792-83c1-6a58209ef49f`; candidate `9a3629eef2ed6f28204f2d13b0b433f8d694a7a9`; bundle `sha256:28ee3673085580af0f8fae92b24b691a1cd79a641bcfe43ca93ff1e0c371e78e`; manifest `sha256:905d84ccdef8c003c2429a2aff67c92a733ce3dff4f2495a82249320801889ab`; sandbox draft PR #5 |
| `M-GCD` | Candidate `f03839e8d9ac22d430b889613fd0f29bf45ed2f1`; bundle `sha256:215bdec5dc8682394429e2bfd1c42f0cb64b7e6c3ca28e3de7d5e81e3943189d`; retained checkpoint `m5/m5-gcd-manual-primary-v1/checkpoint` |
| `O-LCM` | Task `01a0d99e-44f2-74b5-acff-8013d9ac8137`; candidate `a4a515467a3501d486bc8f1ab3bfec3f067af8fc`; bundle `sha256:43a18262c74160759ab68033a3b29c9c7420d277fda541be2a90c152c84a3f6f`; failed manifest `sha256:b308fed4dca615a810d9dd2d80e98647dcdb27c5f15d9d4b702cf1c0e2152069`; no PR |
| `M-LCM` | Candidate `18775d79989f4d86648391d97723393f6c57d0ef`; bundle `sha256:5ea684c701041f46ecded49c40406e74e256b3431fb5c989a8a3761026435b26`; retained checkpoint `m5/m5-lcm-manual-primary-v1/checkpoint` |
| `O-PRIME` | Task `01a0d9a9-91f9-7690-be43-f6a0edf33ef0`; candidate `67add6dbddd1d5600462886d76bbd17c169a220d`; bundle `sha256:911be3137e557ed1ff1c78978a47f19421cf063de400527f930ac00b67c8cbb1`; manifest `sha256:359349e002cb300c6ddb00db62fbf2b96e9479fd8963ee0de1a709e53891bd00`; sandbox draft PR #6 |
| `M-PRIME` | Candidate `f43b0a414d0804c70d07d2168b3652935b91483d`; bundle `sha256:5019bfac570a0ab91edeb86061eecdcf287aef30f9e1ce16dae5b942f9332811`; retained checkpoint `m5/m5-is-prime-manual-primary-v1/checkpoint` |
| `O-SQRT` | Task `01a0d9af-e3a8-759f-b4e6-8337f3caff3d`; candidate `f85f906bc065cb547edf6a8d5b5f21027d0536dc`; bundle `sha256:b1df28fc99044a39d16412a0a2dc86bca2f5e1096553bccc09a91ec46945740b`; manifest `sha256:90eafec0e7a5dc9a8e1f0174a556aa9d1ab3014b947055b843d605483e40e4c5`; sandbox draft PR #7 |
| `M-SQRT` | Candidate `bbb2e8b38f7b73e5b4116693332c995ac4b6d3cf`; bundle `sha256:f8862285e2d2d7c7b98e65c844cfd9f1c407765104e361200139f64226cb60e2`; retained checkpoint `m5/m5-integer-sqrt-manual-primary-v1/checkpoint` |
| `O-MEAN` | Task `01a0d9b2-d415-75b6-a2de-38f8396dcfd2`; candidate `92625851b2040a89e49c0a86912df20ae43e4e03`; bundle `sha256:21a926cbfe04eec9f6f1b129b4632ebb63c582ec9a46d873940dbb69d463f92b`; manifest `sha256:cdb6c5522e04107a064027c25a8c7c96c27feabf902abceffaffc5c3cda3d69f`; sandbox draft PR #8 |
| `M-MEAN` | Candidate `d63b5d3500b6b08e1ddc41a5d3fa0402bbc748fc`; bundle `sha256:7c2eb5a6c2d48a5808ae814ff1975a6b443c01a74925272bdf5b2122080196d6`; retained checkpoint `m5/m5-checked-mean-manual-primary-v1/checkpoint` |

All candidates descended from the frozen start revision and were reconstructed in
fresh host checkouts. The final oracle passed `O-GCD`, `M-LCM`, `M-PRIME`,
`O-MEAN`, and `M-MEAN`. It rejected the other five because the exact frozen
request prohibited test edits; `O-LCM` also failed the committed Ayni quality
contract. No candidate changed policy, dependencies, or the toolchain.

### Aggregate measurements

| Measure | Orchestrator | Manual Ayni | Matched interpretation |
| --- | --- | --- | --- |
| Strict acceptance | 2/5 (40%) | 3/5 (60%) | Orchestrator −1 task, or −20 percentage points |
| Feature behavior plus quality, ignoring prohibited test edits | 4/5 (80%) | 5/5 (100%) | `O-LCM` was the sole quality failure |
| Workflow result | 4/5 draft PRs with final evidence | 5/5 retained candidates; PR publication not part of this arm | Endpoints are not equivalent |
| Run time | total 728s; mean 145.6s; median 149s; range 120–166s | total 396s; mean 79.2s; median 76s; range 46–112s | Per-pair Orchestrator minus manual: 111, 62, 54, 73, and 32s; mean 66.4s, median 62s |
| Timeout/input/setup failure | 0/5 | 0/5 | No primary retry or in-run recovery |
| Fresh validation of product-verified candidates | 2/4 passed; 2/4 failed frozen scope | Not applicable; this arm made no managed verified claim | The two failures were `O-PRIME` and `O-SQRT` |
| Fresh reproduction of strictly accepted candidates | 2/2 | 3/3 | 5/5 accepted candidates reproduced |
| Human effort | Missing | Missing | No active-minute stopwatch; per-accepted-change effort is undefined |
| Provider/compute cost | Missing | Missing | No token or billing telemetry; cost per accepted change is undefined |
| First useful verification | Stage timestamp unavailable | Review timestamp not captured | Cannot compare this measure |

The run-time values are descriptive only. Orchestrator timing includes managed
validation and draft-PR publication, while manual timing stops when the coding
agent checkpoints a candidate; the later manual evidence and review work was not
timed. Therefore the apparent 66.4-second difference is not a performance or
effort comparison.

### Deviations and limitations

- The private oracle initially checked behavior but not the committed Ayni quality
  contract. Commit `59b06f5` added that check after the LCM pair exposed the gap.
  GCD and LCM were revalidated, and the corrected oracle was used for every final
  verdict.
- The private oracle initially did not enforce the already-frozen request's ban on
  policy, test, dependency, and toolchain edits. Commit `eaf7fd9` added exact scope
  checks after all candidates were frozen, and all ten candidates were then
  revalidated uniformly. This post-run validator correction is recorded rather
  than hidden.
- The product's conservative test-edit disclosure marked `O-GCD` even though an
  exact comparison of the sandbox's inline test regions found no test edit. The
  final scope oracle uses the exact region comparison for this fixed repository.
- After the cohort, harness-only resource, output, and environment-variable
  prefixes changed from the milestone identifier to `pilot` so current runtime
  files follow the repository naming policy. Frozen payloads and deadlines did not
  change; the evidence registry retains the original run identities.
- Full model/tool transcripts, token use, provider cost, setup time, review time,
  and active human minutes were unavailable. Manual checkout setup was performed
  by the repeatable launcher outside its clock.
- No first-arm candidate, summary, test output, or PR was inspected before its
  paired second arm terminated. Draft PR visibility remains an operational rather
  than technical concealment control.
- These are synthetic arithmetic tasks in one Rust repository. They provide no
  evidence about demand, pricing, multi-repository operation, or production use.

## Decision

**Iterate within the MVP.** Both arms produced useful, freshly reproducible
changes, but strict acceptance was 2/5 for Orchestrator and 3/5 for manual Ayni.
Orchestrator supplied four evidence-bound draft PRs and caught the LCM quality
failure, but it did not prevent three candidates from violating the frozen test-edit
constraint. Missing human-effort and cost measurements, plus non-equivalent timing
endpoints, make the value comparison inconclusive and rule out **Advance**.

Before any separately authorized cohort, enforce task-specific protected paths
before publication, replace the conservative inline-test heuristic, expose stage
timestamps and provider usage, time manual setup/review work, and compare both arms
at the same reviewable endpoint. Do not add retries or extra tasks to this completed
cohort.

## Completion boundary

All ten primary runs are retained with separate five-task denominators and
matched-pair differences. No missing task, failed arm, rejected candidate, or
missing measurement was converted into success. Applying the
[MVP validation](mvp-validation.md#decision-after-the-pilot) rules, the synthetic
scope and absent monetary ceiling prevent **Advance** or any product-value claim;
the recorded outcome is a workflow-level iterate with an inconclusive value
comparison.
