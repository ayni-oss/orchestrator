# Decision 0002: product name and control-plane model

Status: accepted, 2026-09-17.

## Decision

The product is **Ayni Orchestrator**.

> **Ayni Orchestrator** is a development-workflow control plane that
> coordinates agent-assisted repository changes on controlled infrastructure
> and returns reviewable changes with reproducible validation evidence.

For the MVP, Ayni Orchestrator is application-API-first, not CRD-first:

- Its control plane owns durable project, repository, task, attempt, policy,
  and evidence state in application storage.
- Kubernetes is the controlled execution substrate. The control plane creates
  and reconciles attempt-owned Jobs, Pods, storage, and scoped credential
  references.
- Controllers reconcile the desired state recorded by the control plane with
  observed worker and Kubernetes state. They do not make Kubernetes objects the
  product's primary user interface or source of truth.
- Do not introduce CustomResourceDefinitions (CRDs) in the MVP merely to call
  the product an Operator. Introduce them only when Kubernetes-native,
  declarative configuration is an explicit, tested customer requirement.

## Why this name

The product coordinates a workflow across repository providers, Ayni
environments, agents, workers, validation, evidence, and draft pull requests.
It uses Kubernetes for execution and recovery, but it does not primarily manage
one application through a Kubernetes-native custom API.

In Kubernetes terminology, a controller is a control loop that moves observed
state toward desired state. An Operator is a controller for one or more custom
resources that encodes domain-specific operational automation. The Kubernetes
documentation describes Operators as extensions that use custom resources to
manage applications and components.
[Controllers](https://kubernetes.io/docs/concepts/architecture/controller/)
[Operator pattern](https://kubernetes.io/docs/concepts/extend-kubernetes/operator/)

The proposed MVP has controllers, including environment, workspace, and attempt
controllers, but is not yet a Kubernetes Operator. Calling the product an
Orchestrator describes its user value without precommitting to CRDs or to a
Kubernetes-only interface.

## Initial repository-configuration experience

The initial user interface is a project settings flow, backed by the control
plane API:

1. An organization administrator connects a supported source-control provider.
   For the first GitHub integration, use a narrowly scoped GitHub App
   installation and its selected repository grants.
2. In **Project settings → Repositories**, an authorized project administrator
   selects a granted repository and adds it to a project.
3. The control plane reads provider metadata and the Ayni configuration at a
   pinned revision. It validates the supported project shape and lock, then
   reports `ready`, `setup required`, or `preparation failed` with diagnostics.
4. The administrator selects an approved execution profile and policy. Policy
   sets permitted capabilities, time and resource limits, retry bounds, and
   task-submitter identities; it is not arbitrary repository-authored YAML.
5. The control plane records a repository binding and prepares an environment
   revision that binds the source revision, lock, executor/image identity, and
   preparation evidence.

The user should see provider-derived data, computed readiness, and approved
policy choices rather than a form requiring them to reproduce repository or
Kubernetes configuration manually.

```text
Project
  └── Repository binding
        ├── provider identity and selected repository grant
        ├── observed default branch and pinned configuration revision
        ├── approved execution profile and access policy
        └── environment revisions and readiness evidence
```

Repository bindings are then used to authorize task submission. Each task pins
its starting commit and produces one or more attempts; each attempt owns its
temporary Kubernetes execution resources. A task's history and evidence must
survive deletion or replacement of those resources.

## Terminology

| Term | Meaning in this project |
| --- | --- |
| **Ayni Orchestrator** | The product and its development-workflow control plane. |
| **Control plane** | Durable API, UI, provider integration, policy, task state, evidence retention, and reconciliation coordination. This is not the Kubernetes control plane. |
| **Controller** | A reconciliation loop, such as an attempt, environment, or workspace controller. |
| **Execution resource** | Kubernetes resource owned by an attempt, such as a Job, Pod, temporary storage, or scoped credential reference. |
| **Kubernetes Operator** | A possible future deployment/API model in which Ayni controllers manage Ayni custom resources. It is not the default product label. |

## CRD decision criteria

Revisit CRDs only if a target user needs to declare and review Ayni work through
GitOps or `kubectl`, and the proposed custom-resource API can be the
authoritative desired-state interface without duplicating the control-plane
database. A proposal must define:

- the custom resources, their versioning, status, ownership, and retention;
- how provider credentials and authorization remain scoped;
- how task history and large evidence remain durable outside resource status;
- the relationship between UI/API submissions and declarative submissions; and
- the installation, RBAC, upgrade, and cleanup experience on a real cluster.

Until those conditions are demonstrated, the control plane API and UI remain the
authoritative configuration and task-submission interfaces.

## Consequences and migration boundary

- Product-facing documentation uses **Ayni Orchestrator**. Use **controller**
  for a reconciliation component and
  reserve **Operator** for a CRD-based implementation mode.
- The repository identifier and remote URL remain `ayni-operator` until its
  GitHub rename is complete. The local namespace, image names, deployment
  selectors, Rust package names, imports, and environment variables use
  `orchestrator` now because no compatibility contract has been established.
- M2 continues to choose the durable store, worker transport, and Kubernetes
  resource model. It does not need CRDs to meet its durable-orchestration gate.
