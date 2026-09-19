# First iteration: Rust feasibility and proposed scope

Status: investigation and recommendation, 2026-09-15. Private development and a
Rust preference are user decisions. The runtime topology, resource schema, and
implementation scope below remain proposals; no orchestrator has been implemented.

This document covers the engineering foundation. The later [MVP scope](mvp.md)
defines the full request-to-reviewable-change product, and the
[MVP validation plan](mvp-validation.md) defines pilot readiness and value
assessment. A successful execution spike alone does not complete the product MVP.
The [milestone plan](milestones.md) now controls delivery order and PR boundaries;
the [local deployment decision](decisions/0001-local-kubernetes-deployment.md)
selects Docker Desktop Kubernetes with Kustomize from M0 onward. The worker
experiments below inform that plan without replacing its deployment gates.

## Rust is a supported direction

Kubernetes controllers and Operators can use any language that can call its API.
Kubernetes explicitly lists kube-rs among Operator libraries. kube-rs supplies a Rust client,
controller runtime, and custom-resource derive macro.

Sources: [Kubernetes operator pattern](https://kubernetes.io/docs/concepts/extend-kubernetes/operator/#writing-your-own-operator),
[kube-rs](https://kube.rs/).

Recommend Rust for our controllers and worker application. A small initial stack
would use kube, k8s-openapi, Tokio, Serde, schemars, tracing, and typed errors.
Select compatible versions and a pinned Rust toolchain during implementation.
The kube controller guide uses Tokio; replacing it adds unnecessary work.
[Application guide](https://kube.rs/controllers/application/)

The tradeoff is framework convenience: for example, kube-rs expects explicit
RBAC manifests rather than generating permissions from Rust source. Kubernetes
deployment manifests and permission reviews remain necessary regardless of the
application language. [Manifest guide](https://kube.rs/controllers/manifests/)

Rust ownership applies to our application code. Kubernetes, container engines,
repository toolchains, and optional external coding agents retain their own
implementations. A native agent runtime is a separate future decision.

## First useful outcome

Recommend proving **one pinned repository revision to trustworthy quality evidence**
before adding an autonomous coding loop:

1. Submit an exact source revision and a reference to an approved worker profile.
2. The controller records desired state and schedules a bounded attempt.
3. A worker checks out that revision and uses a pinned Ayni executable to validate
   and build its declared managed environment.
4. Run the unscoped managed repository check and preserve its actual exit status.
5. Collect signals.json and execution.json through the controlled runner, record
   source/environment identities, and expose a result with diagnostic reasons.
6. Restart the controller and repeat events without launching duplicate logical
   attempts. Cancel an attempt and clean up its owned resources.

Use one trusted fixture repository, one namespace, and one controller replica
initially. Manual submission avoids needing a GitHub App before the execution
contract is proven. Source retrieval still needs an explicit access mechanism;
credentials should be injected through scoped references, never resource text.

A tentative namespaced resource such as QualityRun can represent a request and
its attempt status. The name and API group are not selected. Keep immutable
request inputs, desired/observed generation accounting where relevant, owned Job
identity, and distinct infrastructure, quality-failure, cancellation, and success
outcomes. Store bulky logs and evidence separately with references and hashes.

Build a small Rust application with modules for domain state, Kubernetes
reconciliation, Ayni invocation, and artifact storage. Split into crates or
services only when their boundaries justify it. An agent runtime, conversation
database, multi-repo gateway, service catalog, UI, and automatic PR creation can
follow this execution proof.

## The main technical uncertainty: managed execution

Read-only inspection of Ayni checkout
`07474fa3a1c2408f4ed7ffea489ba16323825a88` found:

- The environment library exposes Docker/Podman engine detection, build, doctor,
  preparation, and launch operations. It does not expose a Kubernetes backend.
- A build record binds the lock and preparation to executor provenance and an
  actual engine image ID. A registry digest alone does not establish that state.
- Prepared dependency outputs are materialized under .ayni/environment/ and
  mounted at launch. Image transport alone is insufficient to assume readiness.
- Quality commands create a bounded source snapshot and run in an ephemeral
  writable workspace. Ordinary env run mounts the development checkout instead.
- Managed execution emits execution.json binding the pre-launch build record to
  the quality artifact hash. Host commands invalidate a prior execution sidecar.

Sources in that checkout: ARCHITECTURE.md, environment/src/lib.rs, and
docs/product/environments.md. This later inspection supplements the initial
research baseline in the README; it does not change Ayni.

### Options to investigate

| Approach | What it establishes | Main cost |
| --- | --- | --- |
| Dedicated Linux worker with its own Docker/Podman engine | Can exercise Ayni's existing managed path with no upstream changes | Kubernetes must dispatch to and recover work on that worker; this is not native pod execution |
| Kubernetes Job with an isolated attempt-owned engine | Tests in-cluster lifecycle while retaining Ayni's engine contract | Nested engine, mounts, privileges, networking, and resource enforcement require a real spike |
| Native Kubernetes execution integration | Direct pod execution and Kubernetes resource control | Requires a contract-preserving execution design; not just a new launch command |

Recommendation: use a dedicated disposable Linux worker to establish the managed
baseline, then test an isolated in-cluster worker against that same evidence.
Do not assume a node Docker socket exists or mount a shared host daemon into
repository-controlled work. Keep engine privileges away from the controller.
Worker placement and transport must be selected before implementing the full run
controller; a Job-only lifecycle test does not finish the managed milestone.

If native pod execution reveals a reusable Ayni gap, first record it here. Any
upstream proposal must stand on its own as an Ayni improvement, preserve local
use, and pass Ayni's checks. Keep orchestrator-specific translation and lifecycle
here. Do not label check --host in a prepared pod as a managed check.

## Suggested implementation sequence and exit evidence

1. **Execution spike:** one fixture on a disposable Linux worker; successful
   managed build/check, intentional quality failure, stale-lock rejection, and
   missing-artifact rejection. Record topology and exact versions.
2. **Controller slice:** one request resource and Rust reconciler; real worker
   dispatch, status, artifact collection, bounded retry, cancellation, and cleanup.
   Verify restart and duplicate-event behavior on a disposable cluster.
3. **Development slice:** an attempt-owned editable checkout and one existing
   agent integration. Exercise focused verification, freeze the candidate, and
   run independent final validation before returning a reviewable change.
4. **Prepared environment reuse:** introduce repository readiness and caching
   after the input identity is established; prove that stale preparation cannot
   be reused after relevant dependency or toolchain changes.

For the controller gate, missing evidence must never produce success; quality
failures must remain distinguishable from infrastructure errors; retries must
respect terminal state and immutable inputs. A late completion from a cancelled
or replaced attempt must not overwrite the current result. Cleanup should retain
the final evidence while removing only resources owned by that attempt.

This sequence deliberately validates the execution foundation before the broader
request-to-review workflow. It is a proposal for discussion, not an approved
delivery roadmap or evidence of Kubernetes compatibility.
