# M1 acceptance evidence

The [durable foundation contract](project-workflow-m1-durable-foundation.md#acceptance-and-failure-matrix) defines acceptance. A healthy message-to-response run proves only its named gates. Combine the following runs and retain their outputs before marking M1 complete.

## Reproduce the gates

| Command | Evidence |
| --- | --- |
| `scripts/ci/check-workflow-m1.sh` | Runs the PostgreSQL, integrated canonical gRPC / real broker / retained ledger, and transport gates together. |
| `scripts/ci/check-workflow-postgres-feasibility.sh` | Real PostgreSQL migrations, permissions, admission rollback, FIFO/capacity races, report acceptance, scoped observation, lost notifications, expiry and slow readers. |
| `scripts/ci/check-workflow-jetstream.sh` | Disposable pinned authenticated broker; exact redelivery, split brain, targeted decisions, ACK reconciliation, retention-horizon replay, retirement, role denial, three full streams and maximum body/header. |
| `cargo test --workspace --all-targets` | Contract, ledger crash/reopen, byte conflicts, capacity, ordered report, quarantine and bounded cleanup cases. Ignored infrastructure tests require the wrappers. |
| `AYNI_ORCHESTRATOR_NAMESPACE=ayni-orchestrator-acceptance-RUN dev/ctl acceptance /absolute/new/evidence-directory` | Canonical gRPC, 100 warmed client samples, live watches, every-cursor replay, receipt replay, dependency interruption, process restarts and owned latency gates. |

Deploy the isolated namespace with the current source before the last command. It refuses the normal development namespace. Cleanup retains the namespace, credentials and all authoritative volumes; removal remains an explicit operator action.

## Retained acceptance artifacts

| Artifact | Purpose |
| --- | --- |
| `source-revision.txt`, `source-diff.patch`, `source-manifest.json` | Freeze committed and uncommitted source identities, including new files. |
| `image-identities.json`, `readiness.txt` | Record deployed image IDs and schema checks. |
| `client-latency.json` | Retain 100 distributions, original receipt bytes, command/turn IDs, revisions, update IDs and cursors. No transcript content. |
| `application-metrics.prom`, `executor-health.json`, `owned-latency-gates.json` | Separate owned handoff/watch durations from dependency and client time. |
| `broker-capacity.json` | Record real stream capacity, bytes, consumer lag and retained messages. |
| `pending-before-restart.json`, `pending-after-restart.json` | Prove admitted FIFO work survives a broker outage and application restart before publication. |
| `queue-capacity.json` | Prove 95 admitted calls followed by 10 racing calls commit exactly 100 turns, return five typed capacity failures and select one eligible head. |
| `postgres-unavailable.json` | Prove the unavailable authority returns no false admission receipt or snapshot. |
| `replay-*.json`, `restart-*.txt`, `outcome.json` | Record each retained-state restart, exact receipt replay and cleanup outcome. |

The client binary reports its own unmeasured boundaries explicitly. Its JSON alone does not establish internal timing or the fault matrix. The runner adds server and executor timing evidence before restarts reset process measurements.

## Evidence boundaries

The PostgreSQL suite uses direct store calls for some admission and acceptance races. Executor ledger tests inject crashes by reopening retained SQLite state. The standalone executor broker test uses deterministic application decisions.

The cumulative `durable_handoff` test adds canonical gRPC admission, real PostgreSQL report decisions, retained executor ledgers and real broker/database restarts. Its JSON timeline names 18 exercised boundaries, including failed-predecessor promotion, suppressed decisions, expired storage copies, ACK crash windows and exactly one accepted response. The Docker Desktop runner adds deployed process and dependency evidence.

### Completion

The integrated fixture starts domain work through canonical gRPC and drives actual PostgreSQL, JetStream and executor-ledger handoffs through 18 failure/recovery checkpoints. It suppresses publications and ACKs, reopens ledgers, restarts processes/dependencies, verifies failed-predecessor promotion and frozen context, and checks unique responses and cursor replay.

The expiry fixture demonstrates that JetStream can advance an ACK floor after an unacknowledged decision expires. The application therefore releases a decision only when the latest exact-byte publication remains within its broker retention window. Duplicate storage ACKs and failed retries preserve the original publication time.

These gates establish the deterministic local M1 contract. Shared-environment release, providers, backup/restore, cross-region availability and disk-loss recovery remain outside M1.
