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

This package is not published. Distribution of generated SDKs requires a
separate publication decision.
