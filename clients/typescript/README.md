# Server-side TypeScript gRPC bindings

This private package contains generated native gRPC bindings for
`ayni.orchestrator.v1`. It is intended for server-side clients, including the
browser application's BFF. It must not be imported into browser bundles. The
package is exported only through `@ayni-orchestrator/grpc-bindings/server` under
the Node condition so browser resolution fails closed.

The protobuf sources under `proto/` are canonical. Do not edit `src/generated`;
run `buf generate` from the repository root. The package entrypoint re-exports
the generated `ayni.orchestrator.v1` index, so new v1 source files are included
automatically. Protobuf 64-bit integers are generated as decimal strings to
preserve exact values in JavaScript.

A minimal approved custom-client example lives at
`src/examples/custom-client.ts`. It loads a private CA and credential from files,
uses TLS plus bounded metadata and deadlines, verifies the operator-provided
TLS DNS name even through a loopback port-forward, and resumes a task watch by
resource version with bounded jittered backoff. It is server-side example code,
not a browser entrypoint.

This package is not published. Approved clients receive the canonical private
proto/descriptor bundle described in the
[custom-client guide](../../docs/technical/reference/custom-client-access.md); generated SDK
distribution still requires a separate publication decision.
