# Decision 0002: product name and control-plane model

- Status: accepted
- Date: 2026-09-17

## Historical scope

The product name and application-API-first boundary remain accepted. References to pilot preparation and its resource model are historical; [Decision 0007](0007-clean-v1-runtime-profiles.md) defines the current pre-public v1 architecture.

## Decision

The product is **Ayni Orchestrator**.

> Ayni Orchestrator is a development-workflow control plane that coordinates
> agent-assisted repository changes on controlled infrastructure and returns
> reviewable changes with reproducible validation results.

The product is application-API-first, not CRD-first:

- The control plane owns durable repository, task, attempt, policy, preparation,
  external-action, and result state in application storage.
- Kubernetes is the execution substrate. The control plane creates and
  reconciles bounded, attempt-owned Jobs and their scoped resources.
- Kubernetes objects are not the product's primary user interface or source of
  truth.
- CustomResourceDefinitions are introduced only when a Kubernetes-native,
  declarative interface is an explicit supported requirement.

## Rationale

The product coordinates provider access, prepared Ayni environments, coding
agents, controlled validation, durable recovery, and draft pull requests. It
uses Kubernetes controllers internally, but it does not currently expose an
Operator-style custom-resource API. “Orchestrator” describes the product without
promising Kubernetes-only operation or an unimplemented CRD contract.

## Terminology

| Term | Meaning |
| --- | --- |
| **Ayni Orchestrator** | The product and development-workflow control plane |
| **Control plane** | Durable API, UI, provider integration, policy, lifecycle state, and reconciliation coordination |
| **Controller** | A reconciliation loop for tasks, preparations, Jobs, or external actions |
| **Execution resource** | A Kubernetes Job, Pod, temporary volume, or scoped credential reference owned by work |
| **Kubernetes Operator** | A possible CRD-based interface; not the current product model |

## CRD revisit criteria

A CRD proposal must identify a user who needs GitOps or `kubectl` as the
authoritative desired-state interface and must define:

- resource schemas, versioning, status, ownership, and retention;
- credential and authorization boundaries;
- durable storage for task history and larger artifacts;
- interaction between API/UI and declarative submissions; and
- installation, RBAC, upgrade, and cleanup behavior.

Until then, the application API and browser UI remain authoritative.
