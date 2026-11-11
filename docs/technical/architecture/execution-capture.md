# Execution capture and debug logs

> Legacy command examples below describe historical qualification. Use the
> [single-environment guide](../operations/local-development.md) for current commands.

The worker owns command execution; Kubernetes supervisors own pod lifecycle.
The execution library drains stdout and stderr independently of network delivery,
records action identities and outcomes, and terminates owned process groups on
timeout or cancellation. Existing artifact limits remain artifact validation.

A worker journal records byte chunks and monotonic producer sequences before
sending them over a separate authenticated v1 telemetry stream. PostgreSQL commits
before acknowledging. A retry replays observations, never commands. Raw records
have separate storage and retention from the sanitised JetStream diagnostic outbox.
The trusted workflow API remains loopback-only.

Capture and execution outcomes are separate. Workers allow 60 seconds for final
handoff, with remaining data retained on attempt volumes. Debug capture must not
extend execution authority or prevent urgent cleanup. Deployed qualification is required before claiming the full delivery contract.

## Delivery contract

Debug mode is off by default. It exposes available stdout/stderr and existing
diagnostics from every Ayni-managed component and container through the existing
local console. It does not read arbitrary filesystem logs or unrelated workloads.
Raw debug bytes are not blanket-redacted. Applications must not deliberately log
credentials. Machine-readable verifier stdout and protected artifact files keep
their existing purpose and isolation.

Collection uses the Kubernetes API, without node agents or host mounts. Rotation,
rapid restarts, forced deletion and node failure can make output irrecoverable.
The UI must distinguish known gaps, completeness unknown, pending delivery,
expired history and no emitted output. Repeated identical lines are real records,
not deduplication keys. Combined API logs must not invent stdout/stderr identity.

Initial limits are configurable: 24 hours and 1 GiB retained deployment logs,
128 MiB per worker spool and 1 GiB collector spool. Quota exhaustion continues
execution while recording capture loss. Prometheus pulls operational metrics from
the control plane; no execution identifiers become metric labels.

## Qualification

Delivery is split into supervision/telemetry, complete collection/recovery,
deployment/frontend integration, and deployed acceptance. Unit fixtures establish
local invariants only. Qualification must cover all managed workload types,
restarts, lost acknowledgements, cancellation, rotation and deletion, output
flooding, raw-byte replay, debug-off denial, and browser reconnection.

## Foundation implementation

`backend/libs/execution` owns process groups, concurrent byte drains and SQLite
journals. `backend/libs/telemetry` generates shared types from the canonical v1
protobuf; persistence does not depend on the control-plane API adapter. Runtime
build previews keep their existing bounded sanitised path alongside raw capture.
Protected verification files remain regular files outside repository access.
Trusted launchers retain identity-switch and kill capabilities; executed
repository children must prove empty effective, permitted and inherited sets.

The sender permits at most 32 unacknowledged records on each connection. It waits
for committed acknowledgements for the current window before reading another
fairly selected journal batch, without periodically resending records on a healthy
stream. Acknowledgements must match the outstanding source and sequence. A stalled
acknowledgement stream reconnects after 30 seconds; only reconnection replays the
remaining durable records. PostgreSQL keeps the most recent 64 sequence digests
independently of raw retention, so losing an ACK does
not make an expired payload look like a new record. Conflicting duplicates fail.
A transaction lock orders cursor allocation and commit across producers. Raw
payload quotas expire the oldest records atomically; four log database operations
may run concurrently, separately from workflow writes. A source inventory and
expiry watermark survive payload expiry. Each store and its clones reuse separate
log-reader and log-writer connections, replacing either after disconnection.
Output chunks and history pages do not repeatedly authenticate to PostgreSQL;
reads remain independent of writes, and commits, sequence checks and quota
enforcement remain per record.

The telemetry listener accepts observations only. Existing HTTP fences and
JetStream orchestration still decide execution authority and accepted results.
Loopback debug reads require the deployment flag, and watches stop on server
shutdown. Cancellation notifications can stop processes but never start them.

The foundation alone does not establish complete capture. Collection and frontend
implementation are described below; the [qualification record](../operations/debug-capture-qualification.md) tracks the remaining acceptance gates.

## Collection and recovery

The Rust collector watches only configured release/execution namespaces, checks
managed ownership labels, inventories regular/init/ephemeral containers, follows
current output and attempts previous-instance output. Sources identify pod UIDs
and restart instances; API output is marked `combined`. Timestamp overlap uses
occurrence counts, never text-only deduplication. Reconnection and deletion remain
explicitly uncertain. Journals rotate delivery across producers so a noisy source
cannot indefinitely hide another source's inventory.

Task quality and compatibility parents observe their protected regular files.
Their children receive no telemetry credentials or network permissions. Supervisors
arm retained PVCs with restricted replay pod specifications and discover abandoned
spools after restart. Replay only sends records; it cannot re-run a command or
accept a workflow result. Normal runtime pod retirement allows a final collection
window; cancellation does not wait for logging. Failed replay volumes remain
available for recovery.

## Local debug console

`cargo xtask dev` and `make dev` enable debug collection in the singleton local
deployment. Production charts retain `debug.enabled: false` by default.
The flag gates the APIs, writes, collector
and navigation. Existing lifecycle reporting remains active. Rolling shutdown
closes existing watches; retained rows expire under their configured policy.

Debug → Logs lists retained sources, pages history, follows a resumable cursor,
filters and searches output, copies displayed text and downloads original bytes.
The UI keeps at most 1,000 loaded records, escapes markup and terminal controls,
and never stores raw output in browser persistent storage. Downloads concatenate
the selected original chunks; select a source/stream to avoid mixing byte streams.
A download fixes its upper cursor when it starts. Commit-safe cursors reflect
storage order, not a globally synchronised clock.

## Prometheus aggregation

Prometheus scrapes the control plane's existing `/metrics` endpoint. The
`workflow_telemetry_*` connection/error counters and commit duration observations
are replica-local: aggregate rates across replicas. `workflow_capture_*` values
are shared PostgreSQL projections: use `max` across replicas of one deployment,
never `sum`. Retained outcome and duration projections are gauges over available
history and can decrease on expiry; do not apply `rate` as if they were counters.
Source/execution/pod identifiers never appear as metric labels. A failed projection
read must be treated as unavailable monitoring, not zero outstanding work.
