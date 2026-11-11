# Debug capture qualification

Run deployment acceptance serially against the existing `docker-desktop` context
and `ayni-orchestrator-live` deployment. Start the singleton with `cargo xtask dev`;
the frontend stays at `http://127.0.0.1:5174`, with debug collection and the local
development runtime profile enabled. Do not create a separate acceptance stack.
See the [capture contract](../architecture/execution-capture.md) and
[local development guide](local-development.md).

## Canonical checks

With the repository-pinned Rust and Node tools available:

```text
cargo xtask test deployment debug
cargo xtask test deployment recovery
```

The Rust harness owns scoped fixture pods and serializes deployment acceptance
through the singleton's acceptance lock. Native frontend tests own browser
assertions. The recovery check deliberately restarts the collector while retaining
its PVC. Neither check resets PostgreSQL, JetStream or the collector journal.

The debug check covers regular, init and sidecar registration, failed-to-start
inventory, unrelated-pod exclusion, repeated identical lines, binary bytes,
original-byte download after pod deletion, and frontend history/follow/pause with
inert HTML and terminal controls. The recovery check covers crash-loop restart
inventory, an observed OOM, forced-deletion uncertainty, and retained source
inventory across collector restart.

## Collector backlog regression

The telemetry sender previously resubmitted pending batches every 250 ms on the
same healthy stream. When commits were slower, queued duplicates delayed fresh
records. The sender now keeps one bounded window of at most 32 records, refills it
after committed acknowledgements, and replays unacknowledged records only after
reconnection. A stalled connection is retried after 30 seconds without progress.

The slow-acknowledgement regression commits 96 records with a 25 ms delay per
acknowledgement. The former sender issued 373 requests without draining within
10 seconds; the corrected sender issues exactly 96 and drains in about 2.6 seconds.
The lost-acknowledgement replay regression continues to preserve exact records.

The control plane also opened and authenticated a PostgreSQL connection for each
record. Under local resource limits, live metrics showed commits averaging about
250 ms. History pages also authenticated separately, causing downloads of the
retained backlog to exceed the existing 15-second timeout. Separate dedicated
connections now serve log reads and writes from a store and its clones; workflow
connections remain separate. The PostgreSQL regression verifies reuse, terminates
both connections independently, and confirms reconnection and idempotent replay.

## Singleton qualification, 11 November 2026

Both canonical deployment checks passed after the fixes, with their existing
assertions and deadlines unchanged. The debug check exercised the actual browser
at port 5174. The recovery check restarted the collector and passed crash-loop,
OOM and forced-deletion coverage without creating another environment.

The preserved collector journal started with 6,070 pending records (4,341,486
serialized bytes) and drained to zero after rollout, remaining at zero after the
collector restart. Its cumulative durable
acknowledgements advanced; no journal reset or retained-data deletion was used.
PostgreSQL, JetStream, supervisor ledger and collector PVC identities were
unchanged. A retained-source download that exceeded 15 seconds before reader
connection reuse completed in about 2.1 seconds afterward.

Pinned workspace Clippy and tests, the PostgreSQL replay/reconnection tests,
frontend typecheck/tests/build, protocol generation checks, Helm validation,
security/tooling checks and document validation passed. Managed Ayni quality
completed with all four signals passing against the pinned image and lock.
These results retain the qualification limits below.

## Remaining qualification boundary

These fixtures do not establish complete four-slice acceptance or coverage of
every real workload. Qualify real runtime/task/verification/publication paths,
nested-container cleanup, cancellation and abandoned-spool recovery,
worker/control-plane outages, rotation, eviction and storage exhaustion. Preserve
verifier parsing/isolation and stale-generation rejection while doing so.

Kubernetes API collection cannot guarantee complete output across rotation,
rapid restarts or abrupt deletion. Docker Desktop remains unqualified for
NetworkPolicy enforcement. Neither a passing fixture nor the enabled local
development runtime profile changes those limits.

Earlier dedicated-environment evidence and retired launcher names remain in Git
history. They are not commands or qualification claims for the singleton setup.
