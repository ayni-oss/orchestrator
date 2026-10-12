# Local development

The supported topology uses Docker Desktop Kubernetes with explicit
`docker-desktop` context, a trusted maintainer team and loopback access.
`dev/ctl deploy` builds and digest-pins the worker and installs the local umbrella
chart at `dev/charts/ayni-orchestrator-local`. Production uses the reusable
chart with strict platform scheduling. Local managed runtime defaults to native
`linux/arm64` with emulation disabled; Intel hosts select `linux/amd64` explicitly.

`dev/ctl hot-reload` retains the local Helm release and starts Tilt for server
and web iteration. Dependency, generated-binding and chart changes require
restarting the deployment inputs. Connection Secret rotation reconciles without
restarting Tilt or the control plane. `dev/ctl` never falls back to another
cluster or prints credentials.

Admission freezes runtime/source identities and execution limits. Runtime
configuration defaults to 24 total coding turns and 3 quality repairs, with
repairs bounded to 0–10. Workers attest before checkout, commit coherent work,
run mandatory full checks, repair within budget, bind final evidence and deliver
truthful drafts. Control-plane collection is idempotent and cleanup follows
durable collection.

SQLite schema version 13 migrates recognized versions 6–12 before reconciliation.
The migration preserves connections, certificates, registries, runtime history
and retained tasks, and records obsolete installation resources for restartable
cleanup. Follow [operations](operations.md) and upgrade with cleaned values and
`--reset-values`; do not reset application state or purge the namespace.

Run Rust formatting, Clippy, tests and compilation; protocol generation and
baseline checks; TypeScript/Svelte tests and builds; chart validation; and browser
checks. Fixture-backed UI checks do not replace actual worker-topology evidence.
Exercise managed runtime and publication against the authorized sandbox using
the explicit local context before claiming deployable behavior is validated.

Local worker images package the trusted launcher and checksum-pinned mise.
Repository locks select official Ayni builders, resolved to immutable platform
digests before validation and runtime preparation. Image builds stop on the first
build, inspection or tagging failure, before deployment can attempt to load an
invalid reference.

The worker launcher is built as a static musl executable for the Docker Desktop
node architecture. Its mandatory kernel sandbox must run natively. Use a signed
managed environment matching the node architecture; the Ayni lock supports both
amd64 and arm64. Docker Desktop Kubernetes on Apple Silicon exposes Rosetta's
interpreter through a detached host mount that the sandbox cannot authorize, so
use a native arm64 environment there. Unsupported emulation fails admission.

Local builds check free space on the host volumes containing the repository and
home directory as well as inside Docker Desktop. The VM's virtual free space
does not guarantee that its sparse disk can grow on the host. Builds refuse to
start below the configured capacity threshold; cleanup remains an explicit
operator action and never implicitly removes persistent volumes.

The hot-reload control plane runs under a development-only process supervisor.
Tilt first synchronizes the current Rust source, protobuf definitions and Cargo
inputs, then compiles the binary. Later edits compile and signal only the child
process; they never terminate container PID 1. Restarting the container would
discard synchronized files and the new binary from its writable image layer.
Production images keep their normal direct entrypoint. To refresh an existing
supervised development container explicitly, run
`bash dev/scripts/sync-control-plane-source.sh`. This preserves application data
and credentials and uses only the `docker-desktop` context.


Repository environments follow the same automatic build/certify/publish/verify flow
locally. Ordinary component image packaging remains available for local deployment.
The local chart explicitly selects local-development disposable engines. Official
builders are pulled by resolved digest; no custom builder packaging or image seeding
is required. GHCR push and distinct pull-only credentials, a trusted certificate,
and a compatible committed policy/lock are prerequisites. Always use
`--context docker-desktop` for validation. Deploy cleaned values with `--reset-values`.

Exercise missing-image onboarding, verified reuse, explicit mismatch Rebuild,
uncertain push recovery, and task admission by immutable digest. Inspect workload
specifications for hostPath, privileged access, engine/signing/push credentials in
task or verification Jobs. Never delete retained task data to make validation pass.

## Metrics and verification traces

The local observability stack uses Prometheus for application and kubelet/cAdvisor
metrics, plus an OpenTelemetry Collector and Jaeger for verification traces.
Grafana provides the shared UI: select the Jaeger datasource in Explore for traces
and Prometheus for metrics. The chart no longer deploys kube-state-metrics or
plots provisioned PVC requests; kubelet volume measurements and the existing
Docker Desktop disk fallback remain available.

Tracing is optional diagnostic output. Exporter queues, request timeouts and
shutdown flushes are bounded; Collector outage does not change execution outcomes.
Jaeger stores at most 1,000 traces in memory locally. Logs remain JSON on stdout;
a centralized log backend is not included. See [observability](observability-and-events.md)
for configuration, propagation, privacy and the Docker Desktop validation command.

## Persistent local JetStream

The local chart deploys one NATS 2.15.0 instance with file-backed JetStream.
Its `jetstream-data` PVC requests 2 GiB and survives Helm uninstall; the broker
has a 1 GiB logical storage budget. `jetstream.enabled=false` disables the broker
independently of observability. `observability.enabled=false` removes its exporter
and scrape/dashboard configuration while retaining the broker. The reusable
application chart has no NATS dependency, and application readiness and accepted
results do not depend on broker availability.

A post-install/post-upgrade Job creates or edits `ORCHESTRATOR_LIFECYCLE` in place.
The initial routing subject is `orchestrator.runtime.verification.accepted.v1`,
carrying the structured CloudEvent type
`io.ayni.orchestrator.runtime.verification.accepted.v1` defined in the
[verification contract](verification-events.md). This subject is distinct from
the CloudEvents resource `subject`. D1 provisions transport only; the outbox
publisher and application consumers are subsequent work.

The stream uses file storage, one replica and Limits retention. Defaults are
512 MiB, 100,000 messages, seven days of message age, a 16 KiB message size,
a two-minute deduplication window and at most eight consumers. These are bounded
local development settings, not measured production targets. New publications
are rejected at byte/message capacity rather than evicting retained messages.
Age expiry still removes old messages, including messages a consumer has not read.
The database journal and retained evidence remain authoritative beyond this
replay window. A consumer that falls behind it needs an explicit rebuild path;
broker sequence does not establish application acceptance or execution authority.

JetStream deduplication covers only its configured window. The publisher must
preserve source/ID and immutable bytes across retries; application consumers
must durably deduplicate even after that window. Published outbox and journal
retention must be designed separately from broker retention. No database cap or
acceptance behavior changes in this deployment slice.

Stream deletion, purge and rollup are disabled. Helm provisioning edits existing
configuration without recreating the stream. Lowering limits or message age can
affect retained history, so review such changes before applying them. PVC
expansion depends on the storage class; shrinking claims is unsupported.
`dev/ctl cleanup` preserves the broker claim. Never purge a namespace or delete
its claim to recover from saturation. First stop new publishers, inspect capacity
and replay requirements, then review a retention or capacity adjustment. Restart
the broker with `kubectl --context docker-desktop --namespace NAMESPACE rollout
restart statefulset/jetstream`.

The broker client Service is cluster-internal on port 4222. Its unauthenticated
HTTP monitoring endpoint binds only to Pod loopback; the colocated exporter
exposes metrics on 7777. A NetworkPolicy permits same-namespace Pods labeled
`orchestrator.ayni.dev/jetstream-client=true` to connect and Prometheus to scrape.
Workers receive no broker credentials or trusted-client label. Enforcement
depends on the CNI: Docker Desktop's kindnet setup used for validation does not
provide this isolation. This unauthenticated local profile is for trusted
development only; it does not establish hostile-worker isolation. Before enabling
publishers in an environment that runs untrusted repository code, require tested
network enforcement and authenticated, subject-scoped clients. Production also
requires TLS, replicated storage, backup/restore and availability validation.

Grafana provisions **Ayni Orchestrator JetStream** alongside the existing overview.
It shows exporter reachability, JetStream enablement, broker storage budget and
stream message/byte occupancy. Consumer pending/unacknowledged panels populate
when consumers exist. Outbox age and application handling metrics arrive with
the publisher and consumer. Logical message bytes are not physical PVC usage;
the existing kubelet volume metrics remain useful for filesystem overhead.

Run `dev/ctl test jetstream` on the explicit Docker Desktop context. It creates
an isolated namespace without application workers or credentials, exercises
repeat provisioning, Pod restart, stable-ID deduplication, saturation rejection
and recovery by expanding a tiny test message cap, then checks Prometheus and
the provisioned Grafana dashboard. It uses one fixed disposable test namespace, refuses to overwrite an existing
environment, and deletes that namespace and its test PVC on success or failure.
Normal development release PVCs remain retained. This is broker process-restart evidence, not node/disk-loss,
publisher crash-window or application exactly-once evidence.
