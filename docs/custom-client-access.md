# Custom-client gRPC access

Approved server-side and native clients can use the canonical
`ayni.orchestrator.v1` gRPC API through the opt-in private custom-client
listener. This is not a public endpoint or a multi-tenant boundary. Browser
code must continue through the first-party server-side BFF.

The only supported custom-client product transport is native gRPC over TLS and
HTTP/2. There is no REST mirror, JSON gateway, HTTP transcoding, gRPC-Web
endpoint, or browser-direct contract. Reflection and the unauthenticated standard gRPC health service are disabled on
the protected listener and are not contract or readiness mechanisms. Operators
use the existing HTTP liveness and readiness endpoints.

## Profiles and endpoint discovery

The default Docker Desktop profile is unchanged: it exposes an unauthenticated
h2c listener only on pod loopback for the colocated BFF and host-loopback
`kubectl port-forward`. It rejects a non-loopback value for
`ORCHESTRATOR_GRPC_BIND_ADDRESS`.

Setting `customClients.enabled=true` in the reusable
`deploy/charts/ayni-orchestrator` chart adds a separate TLS-only listener on
port `50052`. The equivalent local-umbrella override is
`ayni-orchestrator.customClients.enabled=true`. It keeps the local listener on
`127.0.0.1:50051`, uses a `ClusterIP` Service, and does not add an Ingress,
NodePort, load balancer, public DNS record, or public package. The included
NetworkPolicy admits port 50052 only from same-namespace pods labelled
`app.kubernetes.io/name=ayni-orchestrator-custom-client`; host access remains an
operator-controlled loopback port-forward. A broader private-network route
needs its own reviewed network policy and endpoint inventory. Public hosting
requires a separate publication and security decision.

Operators provide each approved author with all of the following through an
approved private channel:

- an endpoint URI and expected TLS DNS name;
- the private CA certificate (or an approved enterprise/public trust root);
- a short-lived bearer credential delivered separately from the client bundle;
- the client's capability set and expiry;
- a contract bundle revision and SHA-256 checksum.

Clients must not discover endpoints by scanning a cluster, infer addresses from
reflection, disable certificate validation, or fall back from TLS to plaintext.
For a host-loopback port-forward, use the certificate's DNS name as the HTTP/2
`:authority`; the certificate SAN must contain that exact name. Certificates
need server-auth usage, a current validity period, and a complete chain to the
provided trust root. Operators rotate certificates before expiry and distribute
trust changes out of band; clients do not silently trust a replacement root.

## Configure the protected listener

Create a Kubernetes Secret named `custom-client-grpc-access` in the deployment
namespace with three keys:

| Key | Content |
| --- | --- |
| `tls.crt` | PEM server certificate plus intermediates |
| `tls.key` | matching PEM private key |
| `clients.json` | versioned identities, token digests, expiry, and capabilities |

The identity file has this bounded form (at most 64 clients):

```json
{
  "version": 1,
  "clients": [
    {
      "id": "automation-client",
      "token_sha256": "<64 lowercase hexadecimal SHA-256 characters>",
      "expires_at_unix": 1798761600,
      "capabilities": ["repositories.read", "tasks.read", "evidence.read"]
    }
  ]
}
```

Client IDs are unique 1-64 byte ASCII identifiers using letters, digits, dot,
dash, or underscore. Token digests and IDs must be unique. Generate at least 32
random bytes for each opaque token, deliver the raw value only to that client,
and put only its SHA-256 digest in `clients.json`. The control plane reads the
configuration at startup; rotate or revoke a token by replacing the Secret and
restarting the pod. Expiry is enforced on every new RPC. Never place a raw token
in a manifest, command history, task request, acceptance criterion, idempotency
key, URL, log, or metric.

The chart sets these all-or-nothing variables:

- `ORCHESTRATOR_CUSTOM_GRPC_BIND_ADDRESS`
- `ORCHESTRATOR_CUSTOM_GRPC_TLS_CERTIFICATE_PATH`
- `ORCHESTRATOR_CUSTOM_GRPC_TLS_PRIVATE_KEY_PATH`
- `ORCHESTRATOR_CUSTOM_GRPC_CLIENTS_PATH`

Startup fails if the address is loopback, uses port zero, collides with another
listener, paths are not absolute, TLS material cannot be read, or the identity
file is invalid.
Render the opt-in profile before applying it:

```sh
AYNI_ORCHESTRATOR_DEPLOYMENT_PROFILE=custom-clients \
  scripts/local/render.sh CONTROL_PLANE_IMAGE WORKER_IMAGE WEB_IMAGE
```

Create the namespace and Secret with your normal private secret-delivery
procedure; do not commit generated credentials or keys. Then deploy this profile
instead of the standard chart values:

```sh
AYNI_ORCHESTRATOR_DEPLOYMENT_PROFILE=custom-clients scripts/local/deploy.sh
```

After extracting the verified descriptor bundle and provisioning a client with
`provider_models.read`, exercise the deployed TLS, private Service,
authentication, and canonical contract path:

```sh
export ORCHESTRATOR_CUSTOM_GRPC_CA_FILE="$HOME/.config/ayni-orchestrator/ca.pem"
export ORCHESTRATOR_CUSTOM_GRPC_TOKEN_FILE="$HOME/.config/ayni-orchestrator/token"
export ORCHESTRATOR_CUSTOM_GRPC_TLS_NAME=control-plane.ayni-orchestrator-dev.svc
export ORCHESTRATOR_CUSTOM_GRPC_DESCRIPTOR=./contract/ayni.orchestrator.v1.pb
scripts/local/check-custom-clients.sh
```

The check uses an operator-controlled loopback port-forward, first verifies that
an unauthenticated RPC is rejected, and then lists provider models with the
approved credential. The standard `local` profile remains the default.

## Identities and authorization

Authorization is deny-by-default and evaluated before application dispatch or
resource lookup; transport message-size limits apply during decoding. This
prevents a client from probing resource existence with an operation it cannot
perform. Mutating idempotency keys are scoped to the
principal and RPC method, so equal caller keys from two identities do not
collide.

| Capability | Authorized RPCs |
| --- | --- |
| `repositories.read` | `ListRepositories`, `GetRepository`, `ListPreparations`, `GetPreparation` |
| `repositories.write` | `SaveRepository`, `PrepareRepository` |
| `provider_models.read` | `ListProviderModels` |
| `tasks.read` | `ListDevelopmentTasks`, `GetDevelopmentTask`, `WatchDevelopmentTask` |
| `tasks.create` + `publications.create` | `CreateDevelopmentTask` |
| `tasks.cancel` | `CancelDevelopmentTask` |
| `tasks.retry` + `publications.create` | `RetryDevelopmentTask` |
| `tasks.input` + `publications.create` | `ProvideDevelopmentTaskInput` |
| `evidence.read` | `GetDevelopmentTaskEvidence` |

`publications.create` is deliberately an additional requirement for operations
that can start or resume work which may create a draft pull request. It does not
expose a separate publication RPC.

Send exactly one ASCII metadata value:

```text
authorization: Bearer <opaque credential>
```

The application metadata map is limited to 32 entries and 8 KiB of key/value
bytes, below the separate 16 KiB HTTP/2 header-list guard. `x-request-id` is
optional and remains limited to 1-64 safe ASCII characters. Credentials are
compared by digest in constant time, removed with the request object, and are
never returned, written to the durable task database, or included in logs and
metrics.

## Admission and lifecycle limits

| Boundary | Limit | Failure |
| --- | --- | --- |
| encoded request message | 1 MiB | `OUT_OF_RANGE` at transport decoding; field bounds return `INVALID_ARGUMENT` |
| encoded response message | 4 MiB | `OUT_OF_RANGE` |
| request metadata | 8 KiB and 32 entries, below a separate 16 KiB HTTP/2 header-list guard | `RESOURCE_EXHAUSTED` after gRPC decoding; a larger transport violation can be reset before dispatch |
| concurrent HTTP/2 streams | 32 advertised and admitted per connection | conforming clients queue; load-shed admission returns `RESOURCE_EXHAUSTED` |
| client request rate | 120 admitted starts per 60-second server window | `RESOURCE_EXHAUSTED`; retry after the current window |
| unary deadline | required, at most 30 seconds | `INVALID_ARGUMENT`; an elapsed deadline is `DEADLINE_EXCEEDED` |
| task-watch deadline | required, at most 5 minutes | `INVALID_ARGUMENT` or `DEADLINE_EXCEEDED` |
| task updates | one resumable server stream per watch | reconnect with the last `resource_version` |

Transport overload is never an instruction to retry a non-idempotent request
with a new key. Reuse the original idempotency key and apply capped exponential
backoff with jitter. Cancel a watch when its result is no longer needed. A
watch may end on its deadline, server shutdown, backpressure, or a terminal task
snapshot; reconnect with `after_resource_version` equal to the last complete
update. Point-in-time consumers use `GetDevelopmentTask`.

Canonical status handling:

- `UNAUTHENTICATED`: credential missing, malformed, unknown, or expired; obtain
  a replacement rather than retrying the same value.
- `PERMISSION_DENIED`: authenticated identity lacks an operation capability;
  an operator must approve a capability change.
- `INVALID_ARGUMENT`: a decoded request field or deadline is outside the
  supported contract.
- `OUT_OF_RANGE`: the encoded request or response exceeds its transport message
  limit; reduce the request or use a bounded paginated/streamed response.
- `RESOURCE_EXHAUSTED`: metadata, stream concurrency, or rate limit; reduce load
  or size and retry only when the operation is safely idempotent.
- `DEADLINE_EXCEEDED` / `CANCELLED`: use the durable resource and idempotency key
  to determine the result before retrying.
- `UNAVAILABLE`: reconnect or retry with bounded backoff.

The existing application status table in [the protocol guide](../proto/README.md#errors)
applies after admission. Clients branch on canonical codes and documented rich
error reasons, never message text.

## Contract distribution and upgrades

The protobuf files are canonical. From a clean checkout of a reviewed private
repository revision, an operator builds an immutable bundle:

```sh
scripts/api/build-contract-bundle.sh ./contract-output
```

The bundle contains the v1 proto sources, a standard
`google.protobuf.FileDescriptorSet` named `ayni.orchestrator.v1.pb`, package
compatibility documents, a source revision, and per-file checksums. The script
also writes a checksum for the archive. It does not upload or publish anything.
After explicit approval, distribute the archive and checksum through the
private repository's restricted artifact storage or another specifically
approved private file channel. Do not use a public release, registry, package,
site, or reflection as the supported distribution path.

Pin generated code to the bundle revision and verify both the outer checksum and
`SHA256SUMS` before generation. `ayni.orchestrator.v1` is additive-only: clients
must preserve unknown fields, tolerate additive fields, and handle unknown enum
numbers without inventing meaning. Before upgrading, regenerate in a branch,
run client tests against the target server, and deploy clients before relying on
new optional fields or RPCs. An incompatible contract uses another protobuf
package, such as v2; endpoint aliases or JSON are not compatibility mechanisms.
Operators communicate endpoint, trust-root, credential, and server-version
changes separately. A client must not assume that a reachable endpoint has been
upgraded until the operator confirms its supported package revision.

## `grpcurl` workflow

The protected listener does not expose reflection. Extract
`ayni.orchestrator.v1.pb` from the verified bundle. Store the credential without
putting its value in shell history; for example, enter it at the hidden Bash
prompt:

```bash
install -d -m 0700 "$HOME/.config/ayni-orchestrator"
read -r -s -p 'Custom-client credential: ' ORCHESTRATOR_TOKEN_INPUT; printf '\n'
(umask 077; printf '%s\n' "$ORCHESTRATOR_TOKEN_INPUT" \
  >"$HOME/.config/ayni-orchestrator/token")
unset ORCHESTRATOR_TOKEN_INPUT
chmod 0600 "$HOME/.config/ayni-orchestrator/token"
```

The helper uses grpcurl's environment-header expansion, so the raw value is not
placed in a command-line argument. `uuidgen` creates a distinct safe request ID
for each invocation. Do not enable shell tracing.

```sh
export ORCHESTRATOR_ENDPOINT=127.0.0.1:15052
export ORCHESTRATOR_TLS_NAME=control-plane.ayni-orchestrator-dev.svc
export ORCHESTRATOR_CA_FILE="$HOME/.config/ayni-orchestrator/ca.pem"
export ORCHESTRATOR_TOKEN_FILE="$HOME/.config/ayni-orchestrator/token"
export ORCHESTRATOR_DESCRIPTOR=./contract/ayni.orchestrator.v1.pb

grpc_private() {
  ORCHESTRATOR_TOKEN="$(tr -d '\r\n' <"$ORCHESTRATOR_TOKEN_FILE")" \
  ORCHESTRATOR_REQUEST_ID="$(uuidgen | tr '[:upper:]' '[:lower:]')" \
    grpcurl -expand-headers -max-time 30 \
      -cacert "$ORCHESTRATOR_CA_FILE" \
      -authority "$ORCHESTRATOR_TLS_NAME" \
      -protoset "$ORCHESTRATOR_DESCRIPTOR" \
      -rpc-header 'authorization: Bearer ${ORCHESTRATOR_TOKEN}' \
      -rpc-header 'x-request-id: ${ORCHESTRATOR_REQUEST_ID}' \
      "$@"
}
```

In another terminal, start an operator-controlled loopback port-forward from
service port 50052 to local port 15052:

```sh
kubectl --context docker-desktop --namespace ayni-orchestrator-dev \
  port-forward --address 127.0.0.1 service/control-plane 15052:50052
```

Then list repositories:

```sh
grpc_private -d '{"page":{"pageSize":50}}' "$ORCHESTRATOR_ENDPOINT" \
  ayni.orchestrator.v1.RepositoryService/ListRepositories
```

Submit a task. Use real approved identifiers and keep task content free of
credentials:

```sh
grpc_private -d @ "$ORCHESTRATOR_ENDPOINT" \
  ayni.orchestrator.v1.DevelopmentTaskService/CreateDevelopmentTask <<'JSON'
{
  "idempotencyKey": "client-example-create-1",
  "input": {
    "repositoryId": "sandbox",
    "model": "gpt-5",
    "startRevision": "0123456789abcdef0123456789abcdef01234567",
    "request": "Correct the bounded retry calculation.",
    "acceptanceCriteria": ["The regression test passes."],
    "changeConstraints": {"protectedPaths": [".github"], "allowTestEdits": true},
    "deadline": "3600s"
  }
}
JSON
```

Record `task.id` and `task.resourceVersion` from the response. Observe from that
cursor; use a five-minute grpcurl deadline and reconnect after
`DEADLINE_EXCEEDED` with the last received `resourceVersion`:

```sh
grpc_private -max-time 300 \
  -d '{"taskId":"TASK_ID","afterResourceVersion":"RESOURCE_VERSION"}' \
  "$ORCHESTRATOR_ENDPOINT" \
  ayni.orchestrator.v1.DevelopmentTaskService/WatchDevelopmentTask
```

Cancel with a stable key:

```sh
grpc_private -d '{"idempotencyKey":"client-example-cancel-1","taskId":"TASK_ID"}' \
  "$ORCHESTRATOR_ENDPOINT" \
  ayni.orchestrator.v1.DevelopmentTaskService/CancelDevelopmentTask
```

Retrieve digest-bound evidence after validation:

```sh
grpc_private -d '{"taskId":"TASK_ID"}' "$ORCHESTRATOR_ENDPOINT" \
  ayni.orchestrator.v1.DevelopmentTaskService/GetDevelopmentTaskEvidence
```

The private TypeScript bindings also include a minimal TLS, metadata, deadline,
message-limit, and resumable-watch example at
`clients/typescript/src/examples/custom-client.ts`. Set
`ORCHESTRATOR_GRPC_ENDPOINT`, `ORCHESTRATOR_GRPC_TLS_NAME`,
`ORCHESTRATOR_GRPC_CA_PATH`, and `ORCHESTRATOR_GRPC_TOKEN_PATH`; optionally set
`ORCHESTRATOR_TASK_ID` to exercise resumable watching. The TLS name is verified
and used as the HTTP/2 authority even when the endpoint is a loopback
port-forward. The example is server-side code, is typechecked with the private
workspace, and is not a published SDK.

## Audit and operations

Accepted and rejected requests that reach custom-client admission emit
structured fields for the bounded client ID, RPC, request ID, decision, reason,
canonical status, and latency where available. They never include metadata
values, task content, tokens, token digests, prompts, or authorization headers.
Prometheus counters use only bounded RPC, decision, and canonical-code labels;
client IDs, task IDs, repository names, and request IDs are deliberately
excluded.

TLS handshake failures, HTTP/2 protocol or header-list rejection, pre-dispatch
message-decoding failure, and connection-level stream refusal occur before an
authenticated application admission. They therefore cannot safely carry a
client identity and may not return `x-request-id`; investigate those separately
as transport events rather than attributing them to a configured identity.

Alert on sustained `unauthenticated`, `permission_denied`,
`resource_exhausted`, or `unavailable` outcomes. Correlate an individual call
with the safe request ID in logs, not a metric label. Treat unexpected rejected
calls as a possible credential-distribution or client-configuration incident
and follow the private security-reporting process.
