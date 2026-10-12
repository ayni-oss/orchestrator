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

SQLite schema version 16 migrates recognized versions 6–15 before reconciliation.
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
the CloudEvents resource `subject`. The local chart enables the accepted-event
outbox publisher and the durable verification cleanup consumer.

The stream uses file storage, one replica and Limits retention. Defaults are
512 MiB, 100,000 messages, seven days of message age, a 16 KiB message size,
a two-minute deduplication window and at most eight consumers. These are bounded
local development settings, not measured production targets. New publications
are rejected at byte/message capacity rather than evicting retained messages.
Age expiry still removes old messages, including messages a consumer has not read.
The database journal and retained evidence remain authoritative beyond this
replay window. A consumer that falls behind it needs an explicit rebuild path;
broker sequence does not establish application acceptance or execution authority.

JetStream deduplication covers only its configured window. The publisher preserves
source/ID and immutable bytes across retries, with a SHA256 message ID derived from
an eight-byte big-endian source length, source bytes and event ID bytes. A retry
outside the broker window can create another stream record; application consumers
must durably deduplicate by source/ID. This is at-least-once delivery.

The reusable chart leaves `outbox.natsUrl` empty. The local chart sets it to
`nats://jetstream:4222` and requires `outbox.localDevelopment: true`. These map to
`ORCHESTRATOR_OUTBOX_NATS_URL` and `ORCHESTRATOR_OUTBOX_LOCAL_DEVELOPMENT=true`.
URLs with credentials, paths, query strings or unsupported schemes are rejected.
Disable `outbox.consumerEnabled` and unset the URL to disable delivery, including
when disabling local JetStream.
This opt-in is suitable only for trusted development under the access limits below.

The publisher runs every five seconds, claiming at most eight due records in a
short SQLite transaction before any network wait. Each attempt has a two-second
connect/publish/ACK budget, with persisted exponential retry delay capped at five
minutes. It publishes only the accepted subject and requires an acknowledgement
from `ORCHESTRATOR_LIFECYCLE` with a positive sequence. The outbox stores that
sequence and acknowledgement timestamp. An outage, rejected publication or lost
ACK leaves the accepted result and immutable event pending. A crash after broker
storage but before the SQLite acknowledgement write retries the same message ID.
Database failure is reported with a bounded reason code; broker availability is
independent of readiness and verification acceptance.

Acceptance permits up to 100,000 **pending** outbox records. Published records no
longer consume that admission budget. Reconciliation prunes at most 128 matched
journal/outbox pairs per pass, 30 days after acknowledgement, only after their
preparation and execution are absent from the active cursor. Unpublished events
and active cleanup are never pruned. After pruning, stale cursor/generation checks
reject old verification replays. SQLite can reuse freed pages; pruning does not
shrink the database file. The receipt inbox still has its separate 100,000 retained
observation cap; its retention is not changed by the publisher.

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
when consumers exist. Publisher panels show pending count, pending bytes, oldest
pending age, publication outcomes and retained acknowledged decisions. Logical message bytes are not physical PVC usage;
the existing kubelet volume metrics remain useful for filesystem overhead.

Run `dev/ctl test jetstream` on the explicit Docker Desktop context. It creates
an isolated namespace without application workers or credentials, exercises
repeat provisioning, Pod restart, stable-ID deduplication, saturation rejection
and recovery by expanding a tiny test message cap, then checks Prometheus and
the provisioned Grafana dashboard. It uses one fixed disposable test namespace, refuses to overwrite an existing
environment, and deletes that namespace and its test PVC on success or failure.
Normal development release PVCs remain retained. This is broker process-restart evidence, not node/disk-loss,
publisher crash-window or application exactly-once evidence.


Run `dev/tests/outbox.sh CONTROL_PLANE_IMAGE WEB_IMAGE` with the current production
control-plane image and an existing local web image loaded into Docker Desktop.
It deploys the local chart in the fixed `ayni-orchestrator-outbox-test` namespace,
seeds a synthetic accepted SQLite decision, and checks broker outage, saturation,
a broker-store/local-ACK-write crash window, broker/control-plane restart and
exact event bytes. It verifies the accepted projection remains intact, then
removes the release, namespace and disposable PVCs on success or failure. It
refuses to overwrite a pre-existing fixture. The harness does not delete supplied
image tags; remove test-only host and node tags after its namespace has gone.

### Durable verification cleanup consumer

The local chart enables `outbox.consumerEnabled`; the reusable chart defaults it
to false. This maps to `ORCHESTRATOR_VERIFICATION_CONSUMER_ENABLED=true` and
requires the explicitly trusted local publisher endpoint. Disable this flag before
unsetting the endpoint. SQLite schema 15 migrates existing acceptance and outbox
records unchanged; older binaries cannot open it.

The `verification-cleanup-v1` durable pull consumer filters only the accepted
verification subject, starts at all retained messages, and uses explicit ACKs.
It permits eight outstanding deliveries, one waiting pull, batches of eight or
128 KiB, a two-second maximum pull expiry and a thirty-second ACK deadline.
Processing runs every five seconds with a twelve-second pass budget. Consumer
state uses broker file storage with no inactivity expiry; unlimited redelivery
preserves transient database failure recovery. Existing incompatible consumer
configuration is refused without silently replacing its cursor.

Each typed source/ID is checked against the accepted SQLite journal/outbox.
Within one transaction, the handler persists semantic-content deduplication and
sets the matching accepted cleanup cursor eligible (`next_at=0`), using the
existing committed-cleanup guard. The existing runtime reconciler performs the
cleanup; this consumer never activates a runtime, resets retries or recreates
Jobs. Periodic reconciliation continues even during broker outage or replay
expiry. Superseded or drained cleanup records are committed no-ops.

Only after commit does the consumer send a confirmed ACK. Lost ACK or broker
cursor replay encounters the durable receipt and cannot wake cleanup again.
Malformed, conflicting or unrecognized messages record only a SHA256 fingerprint
and rejection outcome before ACK, without retaining untrusted payloads. Rejection
storage and valid receipts share a 100,000-record cap: when full, delivery pauses
without ACK rather than losing diagnostics. Completed receipts expire after
thirty days, at most 128 per pass, once their accepted preparation and cleanup
have drained. Active cleanup keeps its receipt. Replays after expiry still require the original
accepted journal and current matching cleanup; pruned/unrecognized facts cannot
create work. This retention is not indefinite exactly-once delivery.

Grafana includes application pending, awaiting-ACK and outcome panels. Lag gauges
represent the last successful broker sample; the `deferred` outcome counter
exposes unavailable transport or persistence.

`dev/tests/consumer.sh CONTROL_PLANE_IMAGE WEB_IMAGE` uses the production image,
real SQLite and JetStream in a fixed disposable Docker Desktop namespace. It
checks outage, saturation, committed cleanup wake, malformed rejection, broker
cursor replay and restart while preserving resource projections and later
cleanup backoff. No provider or worker execution is enabled in this fixture.
The namespace, PVCs and release are removed on success or failure; callers must
remove their test-only host and node image tags after the run.
