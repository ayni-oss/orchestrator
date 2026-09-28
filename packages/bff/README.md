# Server-only browser BFF

This private package is the SvelteKit server process's native gRPC adapter. Import
it only as `@ayni-orchestrator/bff/server` from SvelteKit `*.server.ts`,
`+server.ts`, or `$lib/server` modules. It is intentionally unavailable under a
browser export condition.

The package applies loopback-only backend validation, message and deadline
limits, HTTP abort cancellation, safe request correlation, rich gRPC error
translation, and explicit protobuf-to-browser DTO mapping. Decimal `uint64`
values stay strings. Unknown enum values map to `unknown`; generated transport
objects are never returned to browser code.

It is not a generic HTTP/JSON gateway and is not published. Product behavior
continues to belong to the Rust application operations behind the canonical v1
gRPC contract.
