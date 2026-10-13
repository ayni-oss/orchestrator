# Ayni development flow

Product model agreed 2026-10-09. Current behavior and planned extensions are distinguished below.

Ayni Orchestrator turns bounded development requests into code changes, quality evidence,
and draft pull requests. Ayni provides repository environments and actionable quality checks.
The current task uses one repository, one Ayni Managed runtime, and one worker.

## Product ownership

| Concept | Responsibility |
| --- | --- |
| Integration | Access to an external provider. Source-control and registry credentials belong here. GitHub and GHCR are currently supported. |
| Repository | Connected source and branch, committed configuration, and repository-owned runtimes. |
| Runtime | Development environment configuration, image destination, and execution requirements. Ayni settings belong to the Ayni Managed profile. |
| Project | Organizes repositories and their available runtimes for a shared purpose, and owns tasks. |
| Task | Selects a repository and runtime, freezes the inputs and limits, and records work through delivery. |
| Certificate | Signing identity whose public key must be trusted by the repository before its environment is admitted. |

Repositories can belong to several projects. Their runtimes are shared, not copied into each project.
A repository environment change affects future work in every project using that runtime.
Already admitted tasks retain their frozen source, runtime revision, image digest, and limits.

A runtime selects a registry integration and destination; it does not own a second set of registry credentials.
Project membership neither changes repository configuration nor creates a private runtime override.
Custom Runtime is a planned alternative with its own readiness contract and is currently unsupported.

## Runtime preparation

Ayni Managed requires committed `.ayni.toml` and `.ayni.lock`, a supported Ayni contract,
a trusted signing identity, registry access, and a verified image for the execution platform.
Repository access and runtime readiness are separate: incomplete setup stays visible with corrective actions.
Repository owners commit missing configuration and review any certificate-trust PR before it takes effect.

Preparation progresses through **Analyzing → Building → Publishing → Verifying → Ready**.
The repository lock selects supported official builder tooling; the control plane freezes its platform digest.
The builder creates a separate repository image containing locked tools and prepared dependencies.
See [official builders](../reference/repository-builders.md) for execution and credential boundaries.

- An authenticated missing image triggers a build. Access failures and outages remain failures.
- Successful builds publish automatically, then undergo separate pull-only verification.
- Existing images must verify before reuse. Mismatches require explicit Rebuild.
- Source, lock, certificate, or destination changes invalidate incompatible readiness; stale jobs cannot restore it.
- The control plane manages this lifecycle without installing repository environment-building workflows.

## Task development and delivery

Admission freezes source, runtime, model, constraints, coding turns, repair limit, and deadline.
The worker verifies its runtime and checkout, codes with focused Ayni feedback, and commits coherent changes.
It runs a mandatory full repository check and can repair actionable failures within the same limits.
Checks that change publishable source require another commit and check before evidence is finalized.

| Outcome | Meaning |
| --- | --- |
| Execution | Work completed, failed, was cancelled, or exceeded its deadline. |
| Quality | The repository contract passed, failed, or could not be assessed. |
| Publication | A draft was created, publication failed, or publication did not apply. |

Eligible changed work can become a draft with failed or unavailable quality, disclosed explicitly.
Admission failures, unsafe changes, cancellation, and no-change outcomes do not publish.
Publication retry uses retained final work; ordinary task retry creates another attempt.
A delivered draft is not approval to merge, and the product never merges automatically.

Task details use the shared information cards and tables: task status and actions,
draft delivery, attempt progress and model usage, frozen configuration, and quality
evidence. Worker activity expands within each attempt. The task action menu keeps
cancellation, a new attempt, and retrying retained publication distinct.

## Trust and independent assessment

Environment certification, quality results, and merge approval are separate claims.
Verification checks expected identity and trusted signatures, not just the presence of a certificate.
Quality depends on Ayni, the selected tools, and the repository's policy and tests.
See [assurance boundaries](../architecture/assurance.md) for what evidence establishes and what it cannot establish.

An optional independent CI assessment is an agreed direction for customers requiring separate assessment authority.
It must use a fresh candidate checkout and policy, credentials, and retained evidence outside coding authority.
This mode is not implemented; it does not replace the current worker check or revive the retired assessment pipeline.
Its execution design belongs to [research #128](https://github.com/ayni-oss/orchestrator/issues/128).

## Environment-changing tasks

Ordinary tasks currently cannot change `.ayni.toml` or `.ayni.lock`.
The agreed extension allows explicitly authorized native dependency changes, derived lock regeneration,
candidate environment validation, and repository-owner review before merge.
Post-merge reconciliation activates a verified environment for all projects using that repository runtime.

The [environment-change design](environment-changes.md) records the proposed lifecycle and current limits.
[Issue #129](https://github.com/ayni-oss/orchestrator/issues/129) owns design and implementation acceptance.
A successful check in the old environment cannot establish that the proposed environment works.

## Execution research and scope

The current worker owns coding through publication. Separate checkout, coding/check, and publication pods are research alternatives.
A DAG manager is not selected; shared writable data and privileged publication require explicit trust-boundary analysis.
[Issue #128](https://github.com/ayni-oss/orchestrator/issues/128) compares that topology with disposable VM execution and existing reconciliation.

Projects can group multiple repositories, but coordinated multi-repository tasks remain future work.
The current coding adapter uses Rig and OpenAI; broader harness and provider integrations are not implied by the product model.

## Related documentation

- [Operator onboarding](operator-onboarding.md): available setup and task operations.
- [Architecture](../architecture/architecture.md): implemented lifecycle and trust boundaries.
- [Runtime worker](../architecture/runtime-runner.md): budgets, repair, evidence, and delivery.
- [Ayni integration contract](../reference/ayni-integration.md): supported CLI and artifact boundaries.
- [Roadmap](../../ROADMAP.md): remaining work and issue links.
