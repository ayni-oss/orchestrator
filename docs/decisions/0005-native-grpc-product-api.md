# Decision 0005: native gRPC product API

- Status: accepted
- Date: 2026-09-27

## Decision

Ayni Orchestrator's target architecture has one product API: the versioned
native gRPC contract under `proto/ayni/orchestrator/v1`. After migration, the
first-party server-rendered browser application must use that contract through
a server-side BFF, and approved custom clients use the same gRPC services.
Browser code does not call gRPC directly.

The target control plane retains a narrow HTTP operational surface for
liveness, readiness, and Prometheus metrics. Those endpoints are not product
APIs. The product does not provide a parallel REST or JSON contract, HTTP
transcoding, gRPC-Web, or a generic browser API gateway. The former `/v1` JSON routes were an implementation-private migration surface
and were removed at browser cutover; they were never a second supported
contract.

The v1 contract models repository preparation and development work as durable
asynchronous resources. Mutations are idempotent. List methods are paginated,
inputs are bounded, error reasons augment canonical gRPC status codes, and task
updates use a resumable bounded server stream keyed by a durable task resource
version.

Protocol Buffers are canonical across languages. Rust bindings are generated at
build time, server-side TypeScript bindings are reproducibly generated and
checked in, and Buf enforces lint and file-level breaking-change policy. The v1
package is additive-only; incompatible changes require a new package.

## Rationale

One protocol prevents independent transports from drifting in validation,
state, error, and lifecycle semantics. Native gRPC provides generated clients,
streaming, deadlines, cancellation, and a cross-language contract for CLIs,
automation, IDE integration, and alternative trusted clients. A server-side BFF
keeps native gRPC and credentials out of browser code while adapting resources
to page-oriented interactions.

Operational probes and Prometheus scraping have established HTTP integrations
and do not benefit from becoming product RPCs.

## Consequences

- The first-party browser cutover removed the embedded UI and `/v1` JSON product
  routes. HTTP remains only for compatible operational endpoints and a narrow,
  bearer-protected internal publication callback.
- The server must map application errors to canonical gRPC status codes plus
  stable, safe error details.
- Defining this contract does not authorize a namespace-reachable listener. In
  the trusted local profile the native server is required to bind to pod
  loopback. Its declared Service target supports a host-loopback `kubectl
  port-forward`, while normal Service traffic to the pod IP cannot reach the
  listener. Any broader serving boundary requires authentication, authorization,
  transport protection, and verified network isolation.
- The SvelteKit server-side BFF may expose only narrowly scoped same-origin
  server loads, form actions, and bounded browser-native update relays. These
  are private UI implementation details, not a supported Orchestrator API. The
  BFF runs beside the control plane so gRPC remains bound to pod loopback.
- Client authors must tolerate additive fields and unknown enum values and use
  bounded deadlines and stream reconnection.
