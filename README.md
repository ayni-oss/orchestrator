# Ayni Orchestrator

Ayni Orchestrator is a private development-workflow control plane built around
Ayni's environment and quality contracts. It prepares an approved repository
revision, runs a bounded coding agent, validates the frozen candidate
independently, and publishes a reviewable draft pull request.

The private GitHub repository is `ayni-oss/orchestrator`. Ayni remains an
independently useful local CLI in the public
[`ayni-oss/ayni`](https://github.com/ayni-oss/ayni) repository.

## Current capabilities

- a versioned native gRPC product API with health, development reflection, resumable task updates, and an opt-in TLS/authenticated private custom-client listener;
- a deployable server-rendered SvelteKit boundary whose server-only BFF uses generated native gRPC clients;
- durable task and attempt state in SQLite on retained local storage;
- deterministic Kubernetes Job reconciliation, cancellation, and bounded retry;
- GitHub App repository enrollment and draft-pull-request publication;
- content-addressed repository preparation and compatible reuse;
- bounded OpenAI-backed development with a live provider model catalog and retained checkpoints;
- durable per-interaction provider usage, versioned cost estimates, and a local Prometheus/Grafana task and Kubernetes resource dashboard;
- independent Ayni validation of a frozen candidate;
- frozen protected-path and test-edit constraints enforced before publication;
- retained, digest-bound result artifacts; and
- a browser workflow for repository readiness, tasks, recovery actions, and
  results.

## Current operating boundary

The reusable `deploy/charts/ayni-orchestrator` Helm chart contains the product
workloads; the supported `dev/charts/ayni-orchestrator-local` Docker Desktop umbrella adds
version-locked Prometheus, kube-state-metrics, and Grafana dependencies. The
supported deployment is a single trusted-team Docker Desktop Kubernetes cluster. The
internal web and control-plane Services are unauthenticated and
must be reached from the host only through loopback port-forwarding. The
SvelteKit server shares the control-plane Pod so its BFF can reach the
pod-loopback native gRPC listener without making that listener namespace
reachable. An opt-in local Helm value adds a distinct TLS-only native gRPC listener
for explicitly configured custom-client identities. It is capability-authorized,
rate- and size-limited, privately distributed, and remains ClusterIP-only; the
loopback BFF listener and its defaults do not change. See
[Custom-client gRPC access](docs/custom-client-access.md).

The current profiles support the allowlisted private `ayni-oss/sandbox` Rust
repository, one control-plane replica, Ayni `0.13.0`, and local retained
volumes. They do not claim multi-tenant isolation, public hosting, multi-node
storage, general repository support, or production readiness.

## Planning and delivery

Requirements and delivery status live in GitHub rather than repository planning
documents:

- [Ayni Orchestrator project](https://github.com/orgs/ayni-oss/projects/2)
- [Orchestrator issues](https://github.com/ayni-oss/orchestrator/issues)
- [Completed Orchestrator milestones](https://github.com/ayni-oss/orchestrator/milestones?state=closed)
- [Ayni CLI project](https://github.com/orgs/ayni-oss/projects/1)

Issues hold the requirement and acceptance criteria. Pull requests link the
issue with a closing keyword. Priority labels and Project status/iteration fields
provide planning views; Git history and merged pull requests retain
completed-work provenance.

## Documentation

| Document | Purpose |
| --- | --- |
| [Architecture](docs/architecture.md) | Implemented components, state, execution flow, and trust boundaries |
| [Local development quick start](dev/README.md) | Local env files and deploy, check, down, and purge commands |
| [Local development](docs/local-development.md) | Workspace checks, image build, deployment, and API development |
| [Operations](docs/operations.md) | Local access, preparation, updates, retention, recovery, and cleanup |
| [Local Helm deployment decision](docs/decisions/0006-local-helm-chart.md) | Docker Desktop Kubernetes and Helm deployment model |
| [Control-plane model](docs/decisions/0002-product-name-and-control-plane-model.md) | Product name and application-API-first model |
| [Durable orchestration decision](docs/decisions/0003-durable-local-orchestration.md) | SQLite, Jobs, idempotency, and recovery |
| [Preparation and local UI decision](docs/decisions/0004-repository-preparation-and-local-ui.md) | Prepared environments, browser access, and operating limits |
| [Native gRPC product API decision](docs/decisions/0005-native-grpc-product-api.md) | Single product protocol, browser BFF boundary, compatibility, and operational HTTP exceptions |
| [Protocol contract](proto/README.md) | Versioned services, bounds, compatibility, errors, and binding generation |
| [Custom-client gRPC access](docs/custom-client-access.md) | Private endpoint discovery, TLS, identity, capabilities, limits, upgrades, and examples |

Repository development rules and the private boundary with Ayni are recorded in
[AGENTS.md](AGENTS.md).

## Publication boundary

This repository and the Orchestrator Project are private. The Ayni CLI Project
is public with the public CLI repository. Public Orchestrator announcements,
documentation, releases, packages, images, or links from Ayni require an
explicit publication decision. The Apache-2.0 license does not by itself change
repository visibility or establish product availability.
