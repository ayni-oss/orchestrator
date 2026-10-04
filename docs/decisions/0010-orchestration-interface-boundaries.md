# Decision 0010: orchestration interface boundaries

- Status: accepted
- Date: 2026-10-04
- Complements: [Decision 0007](0007-clean-v1-runtime-profiles.md) and
  [Decision 0009](0009-connectors-projects-and-internal-assessments.md)

## Decision

Maintain one application lifecycle for Ayni Managed and Custom Runtime, with
three distinct interfaces: inbound native gRPC for product operations, inbound
server HTTP for operational endpoints and protected protocol-specific callbacks,
and outbound Kubernetes for execution resources. Kubernetes state cannot replace
durable application authority. HTTP and the web BFF cannot create competing
product APIs.

Keep profile-neutral execution contracts in `crates/runtime`, explicit Ayni
contracts and release metadata in `crates/profile-ayni`, and Kubernetes rendering
and operations in `crates/kubernetes`. Generic execution/coding requests carry
an explicit typed profile expectation. Managed requests retain their required
Ayni expectation and serialized shape. Do not add optional Ayni fields whose
absence bypasses managed admission.

The control plane is the composition root. Its server configuration, operational
HTTP adapter and application state are separate modules. HTTP receives only a
read-only projection of the availability it presents. Native gRPC continues to
own capability enforcement, bounds, streaming and public error conversion.

The shared core owns lifecycle and immutable bindings; profiles own runtime
admission and assessment policy. Ayni certification and findings are profile
concerns, not requirements of every custom container. Ayni remains independently
useful; this change does not modify or depend on private additions to Ayni.

## Incremental extraction

Do not move the existing protobuf-coupled store into a crate named "core" and
declare the boundary complete. Introduce application-owned commands, resources
and provider/execution/evidence ports alongside migrated vertical use cases.
Preserve transactional and restart behavior. The public protobuf representation
belongs at the gRPC adapter; retained protobuf payloads can remain an explicit
persistence codec until their schema changes are deliberately designed.

Do not create one orchestration engine per runtime profile, or a generic plugin
framework before the two closed profiles need it. The current Actions importer
is explicitly a managed provider contract. The internal verifier needs its own
pre-PR evidence identity and cannot inherit mandatory workflow/PR fields.

## Consequences and validation

The first extraction preserves the public protocol, database schema, managed
worker JSON and Job manifests. It provides enforceable dependencies and honest
names while leaving existing managed behavior operational. Custom execution,
dynamic connectors and internal verification are separate product milestones.

Test managed wire compatibility and required expectations, a custom typed request
without Ayni metadata, runtime admission, Job isolation, operational HTTP and
gRPC behavior. Enforce crate direction with the existing managed dependency
checks and regenerate the environment lock using Ayni. Runtime changes still
require the explicit Docker Desktop deployment validation boundary.

The [architecture](../architecture.md) documents current ownership and remaining
coupling. Implementation acceptance is tracked in
[issue #98](https://github.com/ayni-oss/orchestrator/issues/98).
