# Local API gateway

`dev/ctl gateway [PORT]` forwards the control-plane Pod's loopback gRPC listener
onto a random host loopback port and starts `workflow-gateway` on
`http://127.0.0.1:3000` by default. Both listener and upstream require loopback IP
addresses. Closing the command stops its port-forward; rerun after a Pod restart.

The adapter serves only allowlisted ProtoJSON requests at `/api/{method}` and
resumable SSE observation at `/watch`. It serves no HTML, JavaScript, CSS or
workspace assets. Other paths return 404. The console and product web UI are
removed; [the former app](../../../examples/web/EXAMPLE.md) is a reference only.

The gateway owns no database, broker connection, workflow state or credentials.
Canonical gRPC owns validation and authority. Protobuf 64-bit fields remain
decimal strings, with ProtoJSON timestamps and enum names.

Host, Origin and fetch-metadata checks, no CORS, no-store responses and the
existing `x-console-request: 1` mutation header protect the trusted local boundary.
The header is retained for existing API tooling; it does not expose a console.
Limits remain 32 unary requests, eight watches, 64-KiB request bodies, and canonical
input/response bounds. This is a local operator adapter, not a remote identity or
authorization system.

Backend API and boundary tests continue to run. Browser and combined deployed
profiles requiring the retired UI fail explicitly before cluster work; historical
browser evidence does not qualify current source. Future UI acceptance must be
rebuilt against current APIs.
