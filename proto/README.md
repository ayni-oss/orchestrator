# Ayni Orchestrator protocol

`ayni.orchestrator.v1` is the canonical target product API contract. Native
gRPC is the supported product transport for repository, provider-model,
development-task, and evidence operations. The deployed SvelteKit server-side
BFF and approved custom clients use the same contract; generated clients and
gRPC transport values must not enter browser bundles.

Existing `/v1` JSON routes are a temporary, implementation-private migration
surface and receive no new product capabilities. In the target architecture,
HTTP remains available only for operational integration: liveness, readiness,
and Prometheus metrics. This contract does not define REST, JSON transcoding,
gRPC-Web, or browser-generated clients.

The supported local server exposes this contract through a pod-loopback Tonic
listener reached with a Service port-forward bound to host loopback. Normal
Service traffic addresses the pod IP and cannot reach the loopback listener.
Any non-loopback endpoint requires authentication, authorization, transport
protection, and verified network isolation appropriate to that deployment
profile.

## Services

| Service | Responsibility |
| --- | --- |
| `RepositoryService` | Repository enrollment plus asynchronous, content-addressed preparation |
| `ProviderService` | Sanitized provider model discovery |
| `DevelopmentTaskService` | Task submission, lookup, actions, lifecycle updates, usage, and evidence |

Mutations carry caller-generated idempotency keys. Keys are scoped to the
authenticated caller and RPC method and are retained at least as long as the
resulting resource. Repeating the same canonical request returns the original
result; reusing a key with different content returns `ALREADY_EXISTS` with
`ERROR_REASON_IDEMPOTENCY_KEY_REUSED`.

`SaveRepository` creates or replaces the enrollment with the same repository
ID. The replacement applies only to future preparation and task submissions;
existing preparations, frozen task snapshots, evidence, and publication results
remain unchanged. Preparation and development submission return durable
resources, normally in a queued state; they never promise synchronous
completion.

List ordering is stable: repository and model IDs sort ascending, while
preparations and tasks sort by creation time and ID descending. Page tokens bind
the original filter and a consistent list snapshot. Tokens are opaque, expire
no earlier than 15 minutes after issue, and cannot be reused with another list
method or filter.

Task lists return bounded summaries. Full lifecycle events, frozen scope,
evidence references, and individual provider interactions are returned only by
task-specific RPCs.

`WatchDevelopmentTask` is the v1 task-update contract. Every durable change to
the represented task, including lifecycle, evidence, publication, and provider
usage changes, increments `resource_version`. Without a cursor the stream first
returns the current task snapshot. With `after_resource_version`, it waits for a
later version. The server ends the stream after a complete terminal snapshot and
may end it earlier for a deadline, cancellation, shutdown, or backpressure.
Clients reconnect with the last received resource version; clients needing only
a point-in-time read use `GetDevelopmentTask`.

## Compatibility

The package release state is recorded in [RELEASES.md](RELEASES.md). An
additive-only package follows these rules:

- never renumber an existing field or reuse a removed field number or name;
- reserve removed field numbers and names in the message or enum;
- do not change field cardinality, scalar type, service name, RPC name, or RPC
  streaming shape;
- add enum values only; every consumer must handle `UNSPECIFIED` and an
  unrecognized numeric value without inventing product meaning (enum additions
  remain potentially source-breaking for exhaustive language-level matches);
- preserve scalar presence when absence differs from zero, false, or empty;
- add behavior behind new fields or methods rather than changing established
  semantics; and
- run the Buf `FILE` breaking check against the base branch for every change.

A future incompatible contract uses a new protobuf package such as
`ayni.orchestrator.v2`; an HTTP alias is not a compatibility mechanism.

## Errors

Clients may send `x-request-id` metadata containing 1–64 ASCII alphanumeric,
dash, underscore, or period characters. The server returns the accepted value
or a generated value in response metadata.

RPCs use canonical gRPC status codes. When a stable explanation is available,
the standard rich status details list contains an
`ayni.orchestrator.v1.ErrorDetail`. Its `reason` is machine-readable; `message`
and metadata are bounded, credential-free, and safe for an operator. The detail
repeats the accepted bounded request-correlation ID when one is available.
Clients must branch on the canonical status and reason rather than matching message
text. Unknown reasons are handled as `ERROR_REASON_UNSPECIFIED` behavior.

| Condition | Canonical status |
| --- | --- |
| Malformed field or exceeded bound | `INVALID_ARGUMENT` |
| Missing repository, preparation, task, or evidence | `NOT_FOUND` |
| Idempotency key reused with different content | `ALREADY_EXISTS` |
| Repository not ready or task phase ineligible | `FAILED_PRECONDITION` |
| Authentication missing or invalid | `UNAUTHENTICATED` |
| Caller lacks an operation capability | `PERMISSION_DENIED` |
| Provider or repository dependency temporarily unavailable | `UNAVAILABLE` |
| Caller or deadline cancellation | `CANCELLED` or `DEADLINE_EXCEEDED` |
| Corrupt retained state or unexpected storage failure | `INTERNAL` |

## Bounds

Servers reject inputs outside these contract limits with `INVALID_ARGUMENT` and
an `ERROR_REASON_INVALID_INPUT` detail.

| Input | Limit |
| --- | --- |
| Encoded request message | 1 MiB |
| Encoded response message | 4 MiB |
| List page | 100 resources; default 50 |
| Page token | 512 bytes |
| Idempotency key | 1–128 ASCII `[A-Za-z0-9._:-]` characters |
| Repository ID | 1–64 lowercase slug characters |
| Git revision | exactly 40 lowercase hexadecimal characters |
| Bare SHA-256 fields | exactly 64 lowercase hexadecimal characters |
| Manifest digest | `sha256:` followed by exactly 64 lowercase hexadecimal characters |
| Provider/model ID | 1–128 safe ASCII bytes |
| Task request | 1–16,384 UTF-8 bytes |
| Acceptance criteria | 1–20 entries, at most 2,048 bytes each |
| Protected paths | 100 entries, at most 16,384 aggregate bytes |
| Follow-up input | 1–8,192 UTF-8 bytes |
| Requested-input prompt | 1,024 UTF-8 bytes |
| Safe diagnostic | 512 UTF-8 bytes |
| Task deadline | 1 second–24 hours |
| Changed paths | 256 entries, at most 65,536 aggregate bytes |
| Scope violations | 512 entries, at most 524,288 aggregate bytes |
| Evidence artifacts | 16 entries; names 1–128 safe relative-path bytes |
| Lifecycle events | 32 per task |
| Provider interactions | 64 per task |
| Error metadata | 16 entries; keys 1–64 ASCII bytes, values at most 256 UTF-8 bytes |

Protected and changed paths are normalized repository-relative UTF-8 paths:
they have no leading or trailing slash, backslash, control character, empty
component, `.` component, or `..` component. A protected directory also protects
all descendants.

Repository identifiers, revisions, digests, model metadata, diagnostics, URLs,
artifact names, and lifecycle reason codes retain the stricter bounds enforced
by the application layer. Generated bindings do not replace server validation.

## Generation and validation

Rust bindings and the descriptor set are generated at build time by
`crates/api` using a Cargo-pinned compiler and generator. Server-side TypeScript
bindings are checked in under `clients/typescript/src/generated` and use a
pnpm-locked local generator. TypeScript represents protobuf 64-bit integers as
decimal strings so counters, costs, sequences, and installation IDs never lose
precision in JavaScript.

From the repository root:

```sh
pnpm install --frozen-lockfile
buf format -w proto
buf lint proto
buf generate
pnpm typecheck
cargo test -p ayni-orchestrator-api
```

Do not edit generated TypeScript. CI regenerates it and rejects a diff. The
first revision that introduces the protocol has no base contract; every later
revision is checked against the base branch.
