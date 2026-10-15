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
3. Run supported `ayni env lock` tooling to regenerate the derived `.ayni.lock`.
4. Install dependencies locally in the worker's working environment and run applicable checks against the proposed source.
5. Create a draft PR containing the source and lock changes with truthful check evidence, then require repository-owner review and merge.
6. On the next repository sync, reconcile the actual merged inputs. Reuse a verified compatible artifact or build, verify, and activate a new shared runtime.

The worker does not build or publish a proposed runtime image before merge.
The plan identifies dependency and environment impact; approval authorizes the affected files and fields.
Unplanned environment requirements return for a plan amendment and approval.
Policy thresholds and trusted keys require separate authorization; derived lock regeneration must not silently change them.

## Local checks and runtime certification

| Pre-merge worker checks | Post-merge runtime preparation |
| --- | --- |
| Use the admitted baseline runtime with dependencies prepared locally | Use the actual merged source and committed environment inputs |
| Record the proposed source and lock separately from the execution runtime | Verify the resulting image identity and immutable digest |
| Establish the reported code-check outcomes within that working environment | Establish readiness for subsequent task admission |
| Do not request image build, signing, publication, or activation | Use the trusted runtime build and publication path when needed |

Evidence binds the baseline source/runtime/policy, proposed lock, local dependency preparation, checked source, and final commit.
Successful local checks do not claim certification of the future runtime image.
If required tooling or checks cannot run locally, the PR reports blocked or unavailable evidence without weakening its requirements.
Image building is not a fallback for unavailable pre-merge checks.

## Recovery and shared impact

A rebase, merge commit, concurrent PR, or trust/destination change requires evaluating the current inputs again.
Superseded jobs cannot activate a newer runtime selection.
Build reuse requires verified equivalence with merged inputs and the expected artifact identity.

If post-merge preparation fails, new work requiring the changed environment stays blocked in every consuming project.
Already admitted tasks retain their frozen inputs.
The UI must expose the failing preparation stage, shared impact, and corrective action without presenting the old image as compatible.

## Design dependencies

- [Execution isolation research #128](https://github.com/ayni-oss/orchestrator/issues/128) retains broader isolation and handoff research for later work.
  A new VM backend or separated task-stage topology is not a prerequisite for this lifecycle.
- [Assurance boundaries](../architecture/assurance.md) separates environment certification, quality evidence, independent assessment, and merge approval.
- [Managed runtime execution](managed-runtime-execution.md) remains the current build and activation contract.
