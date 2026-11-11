# Local development

Run `cargo xtask dev` (or `make dev`) from the Orchestrator checkout and open
**http://127.0.0.1:5174**. This is the only development application. The Rust
controller owns Vite, the loopback gateway at port 3000, port forwarding, image
builds and Kubernetes reconciliation.

Use Docker Desktop with its `docker-desktop` Kubernetes context, Rust 1.94.0,
Node 24.14.0, pnpm 10.31.0 and Helm 4.1.1. Local runtime images currently require
the native arm64 Desktop node. The controller checks these prerequisites and
never falls back to another Kubernetes context, namespace or frontend port.

The application namespace is `ayni-orchestrator-live`. Its `-sessions` and
`-runtime` namespaces isolate managed workloads; they are parts of the same
environment. Database, broker, credentials, execution ledgers and collector
spools persist across controller restarts. Startup does not delete environments.

## Commands

| Command | Result |
| --- | --- |
| `cargo xtask dev` | Start the singleton, or report its current owner and URL |
| `cargo xtask dev status` | Show checkout, branch, component build identities and readiness |
| `cargo xtask dev stop` | Stop owned host processes and application controllers; retain storage |
| `cargo xtask dev redeploy` | Reconcile the same environment again, including after a failed build |

The controller holds a per-user OS lock shared by all checkouts. Stop the current
owner before starting from another checkout. An occupied port is an error with
no fallback URL. The compact development indicator in the frontend shows the
checkout, branch, backend phase and gateway state. Its `/__dev/status` endpoint
exists only on the development gateway and is not a workflow API.

## Editing and recovery

Frontend components and CSS use Vite HMR. Backend, worker, collector and chart
changes are debounced. The controller freezes source files, rebuilds affected
components using persistent compiler caches and checks source identity before
rollout. Edits made during compilation queue another build. Compilation failure
leaves the installed backend available and appears in development status.

New executions receive the updated image identities. Running executions retain
their frozen images, deadlines and authority fences. Gateway forwarding selects
a ready control-plane pod and reconnects when that pod changes. Changes to the
controller itself require `cargo xtask dev stop` followed by `cargo xtask dev`;
status reports this explicitly.

Forward migration edits trigger a build and a read-only retained-checksum
preflight. On success the controller quiesces control-plane admission and runs
the normal migration init container before restoring services. The frontend
stays at the same URL. Changed applied migrations or future schema versions
fail preflight. A migration failure remains visible; repair forward. The
controller never resets data or automatically rolls back a partially applied
schema.

## Debug logs and runtime workers

Development always enables debug capture and the pinned local runtime profile.
Open **Debug → Logs** to inspect managed sources and retained output. Raw debug
output is accessible only through the existing local console boundary. API-only
Kubernetes capture can miss rotation, rapid restarts and abrupt deletion; the
source coverage state records gaps and uncertainty.

The local profile supplies pinned worker, executor and verifier images. Docker
Desktop is **not qualified for NetworkPolicy enforcement**. Enabling runtime
builds here does not establish isolation qualification or change production
chart defaults. Production deployment guidance remains in
[operations](operations.md).

## Validation

Infrastructure and fixture orchestration belong to Rust. Svelte/TypeScript
application tests and browser assertions remain in the frontend. Workload inputs
under `dev/fixtures/*/source` are example repositories, not supporting runners.

```text
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo xtask check tooling
cargo xtask check security
cargo xtask check api
cargo xtask check protocol
cargo xtask check charts
cargo xtask test unit
cargo xtask test frontend
cargo xtask test loopback
cargo xtask test postgres
cargo xtask test jetstream
cargo xtask test durability
pnpm typecheck
pnpm web:build
pnpm docs:check
```

PostgreSQL and JetStream integration checks own disposable fixture containers,
not development stacks. They use public fixture credentials and remove only
the containers they created. Protocol and schema generation use
`cargo xtask generate protocol` and `cargo xtask generate schema --write`.
Regenerate deterministic workload packs with `cargo xtask generate fixtures`.

With the singleton running, deployed checks run serially under a separate lock:

```text
cargo xtask test deployment debug
cargo xtask test deployment recovery
cargo xtask test deployment browser
cargo xtask test deployment hmr
```

Debug/recovery checks create named test pods and remove those exact pods.
Browser acceptance seeds synthetic connection/repository/workspace records in
the canonical database and retains their evidence. It never uses real provider
credentials. UI-only response fixtures use browser interception and start no
servers. The HMR check temporarily edits a component and CSS, restoring their
original bytes in cleanup; do not edit those two files during this check.

`cargo xtask test deployment topology` requires two retained conversation and
task fixtures plus a publisher fixture. It fails when those prerequisites are
missing; an empty inventory cannot prove isolation. The former combined M4/M6/M8
browser harnesses had already been retired. Their historical reports do not
qualify current UI behavior. Backend recovery, publication and admission
contracts remain covered by the Rust PostgreSQL, worker and durable-handoff
suites; new product qualification must name the scenarios actually exercised.

## Transition cleanup

The retired `ayni-orchestrator-capture`, `-capture-sessions` and
`-capture-runtime` namespaces and their retained data were explicitly selected
for deletion. The old 5176 and orphan test frontends were stopped. Normal dev
startup never repeats that discovery or deletion. Keep the live namespace and
its retained storage; do not create a second stack to work around a failure.
