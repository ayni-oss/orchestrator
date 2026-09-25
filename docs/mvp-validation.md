# MVP validation: useful changes with less human intervention

Status: validation plan for the [MVP](mvp.md). No ten-task product-value pilot
has run and no product outcome is claimed. The ten-task pilot comes from
the product discussion. The comparison
protocol and decision rules below are proposed evaluation details to settle
before collecting results.

## Primary validation goal

Determine whether prepared execution and Ayni's feedback loop reduce the human
effort required to obtain an acceptable agent-generated change, while preserving
reproducible validation and understandable failure recovery on team-controlled
infrastructure.

There are two distinct decisions: whether the implementation is safe and complete
enough for the pilot, and whether the observed product value justifies further
investment. A working Kubernetes controller answers only part of the first.

The [milestone plan](milestones.md) maps implementation to completion gates.
Deployable changes are exercised on Docker Desktop Kubernetes. Engineering gates
establish pilot readiness; the ten-task pilot records the product validation
decision.

## Questions and evidence

| Question | Evidence to collect |
| --- | --- |
| Can someone get a useful change through the whole workflow? | Request, attempt history, candidate, draft PR, and maintainer verdict |
| Does preparation reduce repeated setup work? | Initial onboarding effort, cold/warm startup times, setup failures, and manual repairs |
| Does the feedback loop help produce an acceptable result? | Findings, repair iterations, final validation, review comments, and human intervention |
| Can the result be independently checked? | Frozen inputs, managed execution record, complete artifacts, and a fresh validation run |
| Can a maintainer understand and recover failures? | Diagnosis and recovery time for controlled failures, retained checkpoints, and cleanup evidence |
| Is operating the product worth the effort? | Accepted changes, total time/cost including failed attempts, installation effort, and maintainer feedback |

## Engineering gates

Run these scenarios on the actual selected worker and Kubernetes topology before
declaring the MVP pilot-ready. Each scenario needs a recorded expected outcome,
actual outcome, and evidence reference. All applicable gates must pass; missing
or blocked evidence leaves the gate unverified.

| Scenario | Required outcome |
| --- | --- |
| Valid repository and request | Preparation, agent edit, focused verification, final managed check, retained evidence, and one draft PR complete end to end |
| Missing config, stale lock, or unsupported project shape | Actionable setup result; no false readiness or silent policy creation |
| Repeated use of matching preparation | Correct identity is reused; cold preparation also works and is measured separately |
| Changed environment inputs or a failed replacement build | Rebuild or explicit unsupported outcome; stale preparation never satisfies the new request |
| Intentional quality failure | Quality failure is distinct from infrastructure failure; no verified-success claim |
| Missing, malformed, altered, or mismatched final artifacts | Success is withheld even if the process exits zero or a passing file exists in the development checkout |
| Candidate changes after validation | Evidence remains attached to its measured inputs; the new candidate requires validation |
| Agent edits required policy or tests | Changes are disclosed; altered policy cannot silently lower the authorized acceptance standard |
| Repeated scheduling/report events and controller restart | One logical attempt is reconciled; task history survives and repeated reports do not duplicate effects |
| Worker disappears | Useful persisted work is retained; bounded retry or explicit interruption is visible |
| Cancellation or execution limit | Work stops within the configured limit/grace period; a late completion cannot revive or replace the result |
| Uncertain PR creation response | Provider state is reconciled before retry; exactly one intended PR is associated with the task |
| Cleanup with another active attempt present | Only owned expired resources are removed; active work and retained evidence survive |
| Repository access is absent or revoked | Unauthorized new operations stop and the task reports the access issue |

For dependency evolution, declare either rebuild support or explicit rejection
before the pilot and test that contract. An unsupported task must remain visible
in results; it cannot be reclassified after failure to improve the success rate.

Host checks, fake worker reports, and mocked Kubernetes APIs are useful development
tests but do not establish managed execution or cluster compatibility. Artifact
hashes bind records to bytes; controlled collection and validation establish why
the records are trusted. Neither proves that a feature meets its product intent.

## Ten-task pilot

### Prepare the pilot

Use ten real, bounded maintenance tasks from a repository the maintainer actively
uses. Select and record them before execution. Include varied bug fixes, small
behavior changes, and contained refactors within the declared supported scope.
Write acceptance criteria that can be evaluated independently of the agent's
chosen tests. Record known baseline failures or flaky checks before starting.

Record the repository/start revision, agent and model versions/settings, Ayni
version and execution identity, supported project shape, worker topology,
resources, credentials policy, retry/time/spend limits, and cleanup/retention
settings. Secret values do not belong in the pilot record. Freeze the success
definitions and cost ceiling before running the comparison.

### Compare with the existing workflow

For each task, aim for two independent runs: the orchestrator workflow and the same
agent using Ayni with manual preparation and coordination. Use the same starting
revision, request, acceptance criteria, model configuration, and execution budget
where the integration permits. Keep workspaces separate and do not feed one
run's solution or transcript into the other. Alternate run order to reduce bias.

This baseline measures the orchestrator's additional value over the existing Ayni
workflow. If a comparable baseline cannot be run, label those results descriptive
and leave comparative improvement unproven. Report differences in tools, agent
capabilities, or resources that could explain the outcome.

Include initial installation/onboarding effort and failed/cancelled attempts.
Separate cold and warm preparation measurements. Controlled failure scenarios
from the engineering gates are additional exercises, not substitutes for the ten
real tasks. Record retries under the original task rather than counting each as
a new success opportunity.

### Review and reproduce

The maintainer evaluates each candidate against the original acceptance criteria,
records review/rework effort, and accepts or rejects it with a reason. Record
actual merge separately from acceptance; scheduling a merge can be unrelated to
code quality.

For every orchestrator result claimed as verified, repeat final validation from its
frozen candidate in a fresh validation workspace using the recorded declared
environment. Include at least one cold reconstruction without mutable state from
the original attempt. Collect fresh evidence and compare completion accounting
and outcomes. Investigate discrepancies and external/flaky dependencies rather
than assuming identical artifact bytes are required. Unexplained reproduction
failures remain unresolved validation evidence.

## Measurement definitions

| Measure | Definition |
| --- | --- |
| Reviewable-result rate | Tasks producing a draft PR with the required final evidence, divided by all selected tasks |
| Acceptance rate | Tasks accepted against the original criteria, divided by all selected tasks; also report how much human repair was needed |
| Human effort per task | Active minutes spent on setup, intervention, recovery, review, and rework; report those categories separately |
| Human effort per accepted change | Total active minutes across successful and unsuccessful tasks, divided by accepted changes |
| Time to first useful verification | Elapsed time from request submission to the first completed relevant quality measurement; retain failures with no measurement |
| Environment failure rate | Tasks encountering a setup/preparation failure, divided by all selected tasks; include causes and retries |
| Cost per accepted change | Total agent, compute, storage, and attributable operating cost across all attempts, divided by accepted changes |
| Reproduction rate | Verified candidates whose fresh validation succeeds, divided by all candidates claimed as verified; show failed and unattempted counts |
| Recovery and cleanup | Time to actionable diagnosis, safe retry/interruption, and removal of owned resources compared with configured deadlines |

If there are no accepted changes, report that fact and total time/cost; the
per-accepted-change ratios are undefined. Show individual tasks, counts, and
medians with ranges. Keep unavailable measurements explicit. Ten tasks provide
directional learning, not statistical proof or universal performance claims.

## Decision after the pilot

### Advance

The proposed advancement rule is that all engineering gates pass, the pilot
produces maintainer-accepted changes, and matched results show lower human effort
per accepted change without a lower acceptance rate or unresolved evidence
integrity/reproduction failures. The maintainer judges the observed cost and
operating burden acceptable against the ceiling recorded before the pilot.

### Iterate within the MVP

Keep the scope fixed when changes are useful but setup failures, review effort,
cost, or recovery work erase the expected benefit. Address the measured bottleneck
and run another recorded cohort. Insufficient baseline data leaves the value
decision inconclusive rather than automatically successful.

### Reconsider the approach

Reconsider the workflow or execution model if comparable use repeatedly provides
no reduction in effort, if maintaining the platform costs more than the benefit,
or if the selected runtime cannot preserve the evidence contract. More generated
PRs alone do not establish value.

The ten-task sample is the agreed pilot size. No numeric percentage improvement,
acceptance-rate target, cost ceiling, or deadline has yet been agreed. Set any
additional threshold before the pilot and preserve later changes in the record.

## Pilot report

Create a private report for each pilot containing:

1. Date, orchestrator revision, topology, versions, declared scope, and limits.
2. Engineering gate outcomes and evidence references.
3. The ten task definitions, baseline/orchestrator runs, intervention logs, costs,
   candidate/PR references, review verdicts, and reproduction results.
4. Aggregate measurements with denominators and missing-data explanations.
5. The advance/iterate/reconsider decision, rationale, unresolved limitations,
   and the next experiment.

Passing this pilot does not validate multi-repository work, arbitrary languages,
untrusted tenancy, complete Cursor compatibility, market demand, pricing, or a
public launch. Those remain separate questions in the
[broader validation backlog](validation.md).
