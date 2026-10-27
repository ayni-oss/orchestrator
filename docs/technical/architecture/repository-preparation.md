# Shared repository and runtime preparation

Status: repository/runtime preparation remains planned. The [connection foundation](integration-connections.md) implements manual GitHub App setup; catalog and runtime gates remain pending.

## What this decides

- Repository integrations form a shared catalog alongside workspaces. Projects select repositories directly, including reuse across workspaces.
- Onboarding produces verified, immutable runtime artifacts through explicit source, tooling, trust, registry and execution gates.
- PostgreSQL owns commands, receipts, generations and accepted evidence. Scoped workers perform external operations under trusted supervision.
- The new UI ships with each backend slice. Maintainers can inspect and exercise each delivered journey locally.
- Historical `main` behavior informs the contract; its persistence, workers, API and UI are not restored as compatibility layers.

## Why now

The product journey needs a trusted way to prepare repositories and runtime images before tasks can use them.
Current local task preparation asks an operator to supply those inputs manually.
The [roadmap](../../../ROADMAP.md#next-focus-repository-preparation-with-a-usable-ui) makes replacing that manual gap the next focus.

## Current boundary

- Approved tasks retain repository/source declarations; those declarations are not verified source evidence.
- The trusted local helper verifies Git objects and lock bytes, while the operator supplies a runtime digest.
- Prepared attempts retain immutable inputs, but preparation does not establish image availability or compatibility.
- Isolated workers perform separate managed quality; coding, quality and publication remain independent outcomes.
- The local gateway serves APIs only. The new local product supports manual App connections and workspace/project discovery; repository catalog and runtime preparation follow. The retained web example supplies design reference only.

## Ownership and records

| Record | Ownership and meaning |
| --- | --- |
| Integration connection | Shared source, registry or signing capability with safe status and PostgreSQL credential revision references. |
| Repository | Stable provider repository identity and source connection reference, independent of any workspace. |
| Project association | Explicit use of a catalog repository by one project; several workspaces' projects may reference the same repository. |
| Source observation | Exact repository commit, inspected policy/lock bytes or protected references, and their digests. |
| Environment revision | Immutable Ayni contract, lock identity, platform, builder, destination and trust expectations. |
| Preparation operation | Frozen inputs, bounded allowance, generation, stage, workload binding, receipts, findings and cleanup state. |
| Verified runtime artifact | Registry digest and accepted verification evidence bound to the environment revision and inspected source. |
| Attempt selection | Task-owned source, artifact and verification references, budgets and upstream selections frozen for execution. |

An association does not expose credentials, another project's history or integration administration.
Define local-operator grants before exposing mutations; remote tenancy and multi-user access remain separate scope.
Removing an association cannot erase retained task provenance or delete a shared repository.
Removal of catalog records requires dependency checks; external repositories and images need separate deletion authority.

## Preparation gates

```mermaid
---
config:
  theme: neutral
  look: classic
  layout: elk
---
flowchart TD
    A[Connect repository] --> B[Freeze source and inspect Ayni]
    B --> C{Setup and trust valid?}
    C -->|No| D[Retain finding or await reviewed trust change]
    D --> B
    C -->|Yes| E[Resolve approved builder and inspect registry]
    E --> F{Image observation}
    F -->|Unavailable| G[Repair access and retry observation]
    G --> E
    F -->|Confirmed missing| H[Build under bounded preparation grant]
    F -->|Found| I[Verify immutable image]
    H --> J[Certify and publish candidate]
    J --> K[Read back registry identity]
    K --> I
    I -->|Mismatch| L[Retain finding and require explicit rebuild]
    L --> H
    I -->|Accepted| M[Retain evidence and complete cleanup]
    M --> N[Activate exact current selection]
```

| Gate | Required evidence or decision |
| --- | --- |
| Source access | Validate provider repository/installation identity and scoped read access. Connection success does not imply Ayni readiness. |
| Committed inputs | Inspect policy and lock from the same exact commit with bounded reads. Missing or malformed inputs retain corrective findings. |
| Ayni compatibility | Admit an explicitly supported CLI, lock/recipe and artifact contract. Resolve the official builder for that contract/platform and freeze its digest. |
| Full input validation | Use supported Ayni tooling for policy/lock consistency, runtime identity and certification. Parsing an envelope or fingerprint grants no readiness. |
| Configuration | Validate the chosen platform, isolated build capacity, registry destination and enabled signing certificate. Keep current connection status separate from historical probe success. |
| Registry access | Test authentication, push and independent pull authority; retain probe cleanup separately. An outage, denial or rate limit cannot become a missing-image result. |
| Repository trust | Validate the selected public signing identity against committed trust. If changes are needed, separately authorize a narrow PR and inspect the merged source again. |
| Image discovery | Resolve the candidate to an immutable manifest/platform identity. Confirmed absence permits an opted-in bounded build; an invalid existing image requires replacement authority. |
| Build/publication | Invoke Ayni's supported build/certification contract. Separate publication credentials and guard destination replacement; reconcile uncertain publication through readback. |
| Verification | Independently pull the exact digest, verify trusted certification and protected content, and bind fresh managed evidence to source, lock, platform and expected tooling. |
| Activation | Accept only the exact current generation and immutable digest binding. Retain verification before resumable cleanup; pending cleanup cannot appear ready. |

Runtime verification establishes environment compatibility, not passing source quality or merge approval.
A source-only change may reuse an image after applicable verification; a changed environment contract selects new preparation.
Keep desired revision, active verified artifact and historical readiness distinct.
New selection blocks new admissions until its gates pass, while existing attempts keep their frozen inputs.

## Authority and execution

Connection/read, automatic preparation, trust PR creation, replacement, task execution and task publication are separate grants.
Enabling Ayni preparation records explicit resource/time/retry bounds and the permitted registry destination.
General policy bootstrap and unrelated repository edits remain owner-reviewed work.
A retry recovers the same accepted operation; changed inputs or an additional allowance require a new authorized identity.

The control plane admits commands and accepts reports transactionally in PostgreSQL.
Supervisors provision and reconcile narrowly scoped workloads; workers have no database or broker credentials.
Freeze intent before external actions, authenticate workload/generation reports, fence cancellation and adopt only the exact owned workload after restart.
Transport selection follows this authority model; conversation messages do not become build commands.

Separate source-read, signing, push, pull and task-provider capabilities by stage.
Repository-controlled code must not receive signing or push authority through shared processes, volumes or a build engine.
Qualify a bounded build topology and its egress, scheduling, storage and cleanup before enabling it.
Historical privileged-engine deployment modes are research input, not an approved alpha default or proof of isolation.

The supported Ayni onboarding matrix must be qualified explicitly.
Historical `main` admitted 0.15.0; alpha's development CLI pin alone establishes no repository-builder compatibility.
Keep this integration in Orchestrator and consume Ayni's supported CLI/artifacts without reimplementing adapters or lock generation.

## Task handoff and feedback

Trusted server-side preparation resolves the approved repository/ref through its project association.
It verifies exact Git objects, original lock and reviewed upstream ancestry, then produces the existing frozen source handoff.
An arbitrary browser digest or catalog selection cannot attest source or authorize execution.
Preserve current snapshot/context bounds and fail explicitly when the repository exceeds supported limits.

Preparation records the verified artifact/evidence references alongside existing immutable attempt inputs.
Approval still admits awaiting-execution tasks; preparation still dispatches no coding; explicit start still owns execution.
Execution-time checks and credential-free managed quality remain mandatory even after onboarding verification.
Recovery preserves the selected source/runtime and creates fresh linked outcomes under its separate confirmation boundary.

Repository preparation has durable history independent of any conversation and separate from replay retention.
Project blockers link to the shared operation; task failures and questions link back to their originating conversation.
The inspector connects operation, source, environment, artifact, plan approval, task, attempt, provider/tool activity and publication where applicable.
Accepted records explain authority; traces diagnose execution and metrics summarize it without replacing those records.

## Where the detail lives

- [Delivery and acceptance](../research/repository-runtime-workspace-delivery.md) defines UI slices, interruption gates and evidence requirements.
- [Task preparation](task-execution.md), [dependencies](task-dependencies.md) and [quality](task-quality.md) define the existing downstream boundaries.
- [Code organization](code-organization.md) and [tracing](../operations/execution-tracing.md) define current authority and diagnostic responsibilities.
- Historical [onboarding](https://github.com/ayni-oss/orchestrator/blob/c279a7bcdf340f5479cc292c1571bba22af04b27/docs/technical/guides/operator-onboarding.md) and [builders](https://github.com/ayni-oss/orchestrator/blob/c279a7bcdf340f5479cc292c1571bba22af04b27/docs/technical/reference/repository-builders.md) record main's behavior, including its workspace-membership layer, which this design does not retain.
- Historical [activation](https://github.com/ayni-oss/orchestrator/blob/c279a7bcdf340f5479cc292c1571bba22af04b27/backend/servers/control-plane/crates/application/src/runtime_activation.rs) and [runtime evidence](https://github.com/ayni-oss/orchestrator/blob/c279a7bcdf340f5479cc292c1571bba22af04b27/backend/libs/runtime/profiles/ayni/src/managed_runtime.rs) show guarded selection and environment/quality separation.
