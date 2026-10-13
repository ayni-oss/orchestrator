# Reviewed environment changes

Accepted product direction, not implemented. [Issue #129](https://github.com/ayni-oss/orchestrator/issues/129)
owns research, acceptance criteria, and implementation.

## Outcome

An authorized task can update dependencies and propose a matching Ayni environment without weakening its own quality or trust policy.
The repository owner reviews the change; post-merge reconciliation activates a verified runtime shared by all consuming projects.
Ordinary tasks currently protect `.ayni.toml` and `.ayni.lock`; those protections remain in force.

## Proposed lifecycle

1. Admit explicit environment-change scope and freeze the baseline source, policy, runtime, and supported Ayni tooling.
2. Change the native dependency lock, such as `Cargo.lock`, and any required source.
3. Run supported `ayni env lock` tooling to regenerate the derived `.ayni.lock` and create a draft PR containing both changes.
4. Build an isolated candidate environment for the exact proposed revision and run full candidate checks inside it.
5. Require repository-owner review and merge; never activate the candidate merely because its build succeeded.
6. Reconcile the actual merged revision, build or safely reuse an equivalent artifact, verify it, and activate the shared runtime.

A check inside the old runtime cannot validate the new locked environment.
If candidate execution is unavailable, the PR must report that limitation and remain blocked where validation is required.
Policy thresholds and trusted keys require separate authorization; derived lock regeneration must not silently change them.

## Candidate and active environments

| Proposed candidate | Approved active runtime |
| --- | --- |
| Bound to an exact PR revision and proposed lock | Bound to accepted repository inputs and verified digest |
| Isolated validation with candidate-scoped authority | Available for new task admission under repository trust policy |
| Cannot request the normal signing seed or publish trusted default-branch identity | Uses the reviewed trusted build and publication path |
| Retained or cleaned up under a candidate policy | Reconciled for every project using the repository runtime |

Research must choose candidate signing, separate trust, or unsigned isolated verification before implementation.
The build mechanism may be reusable; the authority and activation rules must remain distinct.
Moving a tag does not establish that a candidate artifact matches the merged source and inputs.

## Recovery and shared impact

A rebase, merge commit, concurrent PR, or trust/destination change requires evaluating the current inputs again.
Superseded jobs cannot activate a newer runtime selection.
Build reuse requires verified equivalence with merged inputs and the expected artifact identity.

If post-merge preparation fails, new work requiring the changed environment stays blocked in every consuming project.
Already admitted tasks retain their frozen inputs.
The UI must expose the failing preparation stage, shared impact, and corrective action without presenting the old image as compatible.

## Design dependencies

- [Execution isolation research #128](https://github.com/ayni-oss/orchestrator/issues/128) defines safe candidate execution and handoff options.
- [Assurance boundaries](../architecture/assurance.md) separates environment certification, quality evidence, independent assessment, and merge approval.
- [Managed runtime execution](managed-runtime-execution.md) remains the current build and activation contract.
