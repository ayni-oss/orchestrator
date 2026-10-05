# Architecture

Ayni Orchestrator has **one orchestration lifecycle with separate interfaces and
runtime profiles**. The product journey is **Connectors → Projects → Tasks**.
[Decision 0009](decisions/0009-connectors-projects-and-internal-assessments.md)
defines the agreed product; [Decision 0010](decisions/0010-orchestration-interface-boundaries.md)
defines the code boundaries. These decisions describe the target, not a claim
that every product flow is implemented.

The project foundation provides named, single-repository projects with
project-owned tasks, immutable repository membership, and archive history. The
remaining connector, project-readiness, environment-production, and internal
assessment flows are target behavior; current implementation ownership and gaps
are tracked in [the product epic](https://github.com/ayni-oss/orchestrator/issues/90).

## Three interfaces, different responsibilities

| Interface | Direction and contract | Owns |
| --- | --- | --- |
| Native gRPC | Inbound, versioned user contract in `proto/ayni/orchestrator/v1` | Authentication, capabilities, request bounds, protobuf conversion, status mapping, pagination and watch transport |
| Server HTTP | Inbound, server-specific operational and browser integration endpoints | Liveness, readiness, metrics; protected provider callbacks and one-time credential exchange when those flows are implemented |
| Kubernetes | Outbound, internal execution adapter using the Kubernetes API | Deterministic Jobs and related resources, isolation, credential references, scheduling, observation and cleanup |

The gRPC API is the product API. The server-side web BFF adapts it to browser
forms and sessions; it does not implement another lifecycle. HTTP callbacks
must enter the same application use cases after protocol-specific validation.
Their bounded secret-bearing bodies must not enter logs, caches or ordinary
resource responses. Operational readiness is distinct from a project's
readiness to admit a task: connector configuration must remain possible when
execution infrastructure is unavailable.

Kubernetes Jobs are disposable execution resources. They are not user-facing
product resources or a second database. Durable application state decides
admission, retries, cancellation, readiness and publication. Job completion is
an observation, not proof that an environment or candidate is valid. This
design does not require product CRDs or expose Kubernetes objects through gRPC.

## Shared orchestration and profile policy

The core owns connector/project identity, repository membership, immutable
profile revisions, task admission, attempts, deadlines, budgets, cancellation,
candidates, evidence binding, publication and reconciliation. A project has
exactly one repository in this release. A connector can serve many projects.

Both profiles use the same task and candidate lifecycle, persistence,
Kubernetes execution adapter and provider publication path. A profile adds
explicit admission and verification policy; absence of Ayni metadata must
never be interpreted as permission to skip a managed check.

| Shared responsibility | Ayni Managed policy | Custom Runtime policy |
| --- | --- | --- |
| Admit an immutable OCI digest and platform | Verify committed Ayni inputs, producer trust and certificate through supported Ayni contracts | Validate the supplied container against the supported launcher, platform and isolation contract |
| Resolve the execution environment | Discover, or internally build a genuinely missing certified environment | Resolve and verify the owner's supplied image; no Ayni build or signing requirement |
| Develop and checkpoint immutable candidates | Make Ayni available as development feedback | Run the repository's supported native tooling without requiring Ayni |
| Bind assessments and evidence to the exact candidate | Independent internal Ayni verifier and bounded repair policy | Explicit custom assessment policy; do not label unassessed work as Ayni-verified |
| Publish and reconcile review objects | Present bounded Ayni findings and actual final outcome | Present the actual custom assessment or unassessed outcome |

Ayni owns lock interpretation, policy evaluation and certificate verification.
Orchestrator owns execution identity and verifies that observations and evidence
belong to the operation being reconciled. Shared scheduling and publication
must not acquire mandatory Ayni lock, signal or GitHub workflow fields.

## Product flow and trust

Connector setup validates a GitHub App and persists its operational credentials
with authenticated encryption. The encryption root remains in an existing
operator-managed namespace Secret. Registry pull/push, producer signing,
build-engine and model credentials remain deployment Secret references, not
connector records. Installation discovery can refresh without recreating a
connector. Disabling one prevents new provider operations and degrades affected
projects without deleting history.

Project inspection is tied to an observed default-branch commit. For Ayni
Managed it checks access, committed inputs, compatibility, producer trust,
owned integration files and the environment. Missing Ayni inputs require owner
action. An idempotent setup PR changes only owned integration files when needed;
the target does not install GitHub build or assessment workflows. Reinspect
after merge before advancing readiness.

Environment discovery uses the deterministic identity and private
`ghcr.io/<owner>/<repository>-ayni` package. Only confirmed absence authorizes a
deduplicated internal build intent. Access errors, invalid signatures, drift
and uncertain publication require remediation or reconciliation. Independent
verification binds the platform-specific manifest digest before admission.
Source-only changes can reuse the same environment after verification.

Task admission freezes repository/source, profile revision, digest/platform,
constraints, deadline and model budget. The agent develops in that image and
saves a checkpoint. The target Ayni verifier uses a fresh checkout of that
candidate in the same image, retaining evidence outside candidate-writable
storage. Findings allow at most three repair cycles within the original limits;
every repaired commit needs a new assessment. Candidate policy/lock changes
that invalidate the frozen environment require owner review rather than
automatic replacement certification.

Evidence binds project/task/attempt, base and candidate commits, verifier Job,
profile revision, runtime digest, lock/configuration, platform, Ayni version and
artifact digests. Valid work may become a draft with passed, failed, incomplete
or environment-update-required status. Cancellation blocks subsequent automatic
publication; no changes or no valid candidate produce no PR. Changed owned PR
heads invalidate earlier assessment authority. Reassessment does not silently
start another agent repair session.

External mutations have durable intent and deterministic identities before
execution. After an uncertain response, reconcile existing Job, registry or
provider state before retrying. Preserve task history and frozen digests across
profile changes. Coding workloads receive no Kubernetes token, host socket,
controller database, signing key or registry-push credential. Checkout and
model credentials have separate workload scope; the verifier has neither model
nor signing/push credentials.

## Code structure

| Location | Current ownership |
| --- | --- |
| `proto/`, `crates/api`, `clients/typescript` | Public gRPC schema, generated bindings and request syntax validation |
| `crates/domain` | Shared change constraints, scope bounds and usage values |
| `crates/runtime` | Profile-neutral typed execution/coding contracts, activity, candidate reports, OCI/launcher admission and bounded input reading |
| `crates/profile-ayni` | Explicit managed execution types, Ayni expectation/result validation, release pin, and the existing Actions assessment contract |
| `crates/kubernetes` | Execution-only deployment/workload configuration, neutral Job construction, submission, observation and cleanup |
| `crates/control-plane/src/application.rs` | Shared application state and durable reconciliation entry point |
| `crates/control-plane/src/configuration.rs` | Server process/listener configuration |
| `crates/control-plane/src/runtime_configuration.rs` | Runtime JSON admission, reconciliation scheduling, managed image derivation and publication policy; explicit mapping to Kubernetes inputs |
| `crates/control-plane/src/http.rs`, `http/` | Operational router and health JSON using only projected model availability |
| `crates/control-plane/src/grpc*` | Native user-facing transport, access enforcement and response mapping |
| `crates/control-plane/src/clean_store*`, `runtime_reconciler*` | Existing durable lifecycle and reconciliation, still coupled to protobuf resource types |
| `crates/control-plane/src/runtime_source*`, `onboarding*` | Existing GitHub integration and managed onboarding |
| `crates/worker` | Executable launcher, checkout, agent and explicit managed verification/build operations |
| `packages/bff`, `apps/web` | Server-side browser adapter and UI |

Dependencies point from adapters toward the contracts they implement. The
runtime crate cannot depend on Ayni, Kubernetes, gRPC or executables. The Ayni
profile can depend on shared runtime contracts; Kubernetes can depend on both
when rendering managed workloads. Neither the worker nor Kubernetes depends on
the public API. The control-plane executable composes those pieces. The
repository's Ayni dependency policy enforces these crate boundaries.

Managed requests instantiate the generic execution/coding contract with a
required `RuntimeExpectation`. This preserves the serialized managed request
shape and fail-closed validation. A typed custom request can use its own
admission data without importing Ayni. That separation alone does not enable
custom execution in the current reconciler or worker.

## Implementation limits and next boundaries

The current managed path still uses repository-first onboarding and Actions
assessment collection. Its assessment contract requires a PR and workflow/run
identity; it must not be reused as the pre-publication internal verifier
contract. The connector, project readiness, internal environment production and
repair flows above remain tracked by [the product epic](https://github.com/ayni-oss/orchestrator/issues/90).
The project foundation is implemented with single-repository membership and
project-owned tasks; connector management and project readiness remain follow-up
work. [Custom Runtime #66](https://github.com/ayni-oss/orchestrator/issues/66)
remains unimplemented end to end.

Application logic still reads generated protobuf resources, stores protobuf
payloads and calls concrete providers. Do not describe this as a fully isolated
core. Extract real use cases with their commands, results and ports into an
application crate as they are changed: connector operations, project inspection,
environment resolution, task admission, then assessment/publication. Keep wire
conversion at gRPC and explicit persistence codec boundaries. Move managed
evidence rules out of generic storage while preserving shared identity and
digest checks. Introduce explicit profile dispatch before admitting custom
tasks. Coding, build and verification Jobs already share neutral workload
construction rather than deriving one operation from another's manifest.

The Kubernetes adapter receives only execution settings. Application scheduling,
publication policy and managed GHCR source derivation live in the control plane.
Runtime JSON retains its strict format and is mapped explicitly into adapter
inputs. The digest-pinned `runtime.workerImage` chart setting is the only worker
image setting; the namespace comes from the Helm release.

The supported deployment remains Docker Desktop, one trusted team, loopback
access and one controller replica. This structural change does not alter the
protobuf contract, persisted schema, managed JSON format or rendered workload
isolation. Durable architecture belongs here; acceptance and delivery status
belong in the linked issues.
