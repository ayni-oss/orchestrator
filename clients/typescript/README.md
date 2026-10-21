# Server-side TypeScript gRPC bindings

This private package contains generated native gRPC bindings for `ayni.orchestrator.v1`. It is a contract artifact for trusted server-side tooling and must not be imported into browser bundles.

The protobuf sources under `proto/` are canonical. Do not edit `src/generated`; run `buf generate` from the repository root. The package entrypoint re-exports the generated `ayni.orchestrator.v1` index. Protobuf 64-bit integers use decimal strings to preserve exact values in JavaScript.

M1 Slice 2 workflow services are available only on the control-plane pod's loopback listener. These bindings do not imply a supported custom-client, browser, or published SDK surface. External distribution requires a separate contract and security decision.
