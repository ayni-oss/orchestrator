# Code organization

Reusable contracts and infrastructure primitives live in `backend/libs`.
Server policy and Kubernetes coordination stay with the control plane; trusted
process execution and publication stay with the worker.

| Owner | Responsibility |
| --- | --- |
| `backend/libs/oci` | Provider-independent OCI syntax and canonical SHA-256 identities |
| `backend/libs/providers` | Enabled Rig provider client construction shared by project and coding agents |
| `backend/libs/domain` | Domain identities, content hashing and model usage |
| `backend/libs/events` | Protobuf lifecycle payloads and CloudEvents structural validation |
| `backend/libs/registry` | Bounded registry credentials and scoped registry requests |
| `backend/libs/observability` | Logging, redaction, tracing and bounded exporter shutdown |
| `backend/libs/runtime/contracts` | Profile-independent worker requests and outcomes |
| `backend/libs/runtime/profiles/ayni` | Ayni compatibility, frozen requests and evidence validation |
| `backend/servers/control-plane/crates/application` | Application policies, ports and durable preparation vocabulary |
| `backend/servers/control-plane/crates/grpc-contract` | Generated product API bindings and boundary validation |
| `backend/servers/control-plane/crates/kubernetes` | Owned workload manifests, observations and cleanup |
| `backend/servers/control-plane` | Composition, provider adapters, SQLite persistence and coordinators |
| `backend/workers/runtime-worker` | Trusted supervisor, sandbox, coding, evidence retention and publication |
| `clients/typescript` | Generated TypeScript protocol bindings |
| `packages/bff` | Trusted operator transport and DTO mapping |
| `apps/web` | Operator views and browser resource invalidation |

## Choosing a boundary

Move code into a shared library when multiple owners need the same contract or
mechanism and it can operate without importing server or worker policy.
Canonical digest validation uses the OCI parser through existing domain/runtime
interfaces. Profile-specific checks still decide whether an otherwise valid
identity is admissible.

Keep admission, desired-state guards, provider side effects and database
transactions in their owning application or adapter. Kubernetes code remains
server-owned because it implements the control plane's execution ports. The
product API crate remains at the server boundary; sharing a schema does not make
server policy reusable worker code.

`.ayni.toml` enforces forbidden dependency edges. Changing a library boundary
requires updating those edges, Cargo manifests, build paths, generated consumers
and managed locks together. Orchestrator functionality stays here; Ayni upstream
changes require a separate independently useful CLI use case.

## Operator feature modules

The web application owns task submission parsing, bounded choice loading and SSE
response lifecycle modules under its server library. Routes keep resource scope,
authorization and cursor semantics explicit. Feature components consume named
view models and BFF DTOs rather than generated route data types. Task identity and
execution/quality/publication summaries share presentation components.

One browser refresh coordinator coalesces resource and conversation invalidations,
keeps a trailing read for changes during a request, and defers background work
while editors or dialogs are active. The selected conversation has one scoped
stream; the resource stream remains shared. Periodic repair runs every 120 seconds
for project conversations and tasks.

## Agent configuration and protocol ownership

`AgentProfileService.GetAgentCapabilities` exposes the runtime contract's role
catalogs, defaults and limits on the operator listener. Project/workspace detail
responses include `effective_agent_defaults`, resolved by the same backend policy
used for admission. These response-only projections are never persisted as raw
overrides. The BFF keeps configured, inherited and effective values separate; the
browser has no independent capability catalog or inheritance implementation.

Product RPCs use `ayni.orchestrator.v1`; event messages use
`ayni.orchestrator.events.v1`. Their source directories mirror these packages.
The gRPC method path is `/fully.qualified.Service/Method`, independent of local
checkout names. Generated TypeScript files come from `buf generate` and remain
inside this repository's `clients/typescript` package.

## Retained compatibility

Worker evidence synchronization projects accepted attempts directly and never
queues automatic model-summary turns. Stored summary tables and retirement
migration remain for historical records. Legacy proposal receipts, frozen
one-shot planner requests and historical event readers preserve their documented
replay/recovery contracts; new research uses resident sessions. Frozen profiles
validate their recorded instruction text and content hash, so newer built-in
instructions never rewrite admitted work. Retiring these readers requires a
separate compatibility-window decision.
