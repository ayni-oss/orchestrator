# Execution tracing

Tracing explains execution across process boundaries. PostgreSQL receipts, accepted
activity and task evidence remain the authority for work and outcomes. Export is
optional; collector failure, dropped spans or sampling cannot cancel accepted work,
authorize retries or replace project accounting.

## Local collection

Run the supported Docker Desktop deployment with the optional collector:

```sh
AYNI_ORCHESTRATOR_OBSERVABILITY=true dev/ctl deploy
```

The local chart enables `ayni-orchestrator.observability.collector.enabled`. The
reusable chart also accepts that setting directly. It deploys the digest-pinned
OpenTelemetry Collector 0.147.0 with an internal `workflow-traces` Service and OTLP
HTTP on port 4318. The release's control plane, supervisors, session workers and
task/publisher processes receive the scoped exporter endpoint. Session/task egress
allows that same-release collector only; the collector has no external egress.
There is no public collector endpoint or proprietary observability requirement.

The collector rotates OTLP JSON files at 8 MiB with two backups and one-day age
retention, within a 32-MiB `emptyDir`. Its memory limiter and bounded batches keep
this a local diagnostic profile. Files disappear when the collector Pod is
replaced; export evidence before that operation. Application and worker restarts
do not replace the collector. This is not a production trace retention service.

For an existing self-hosted collector, set `observability.tracesEndpoint` explicitly
in the reusable chart. The endpoint must be an HTTP(S) OTLP trace receiver without
embedded credentials, query parameters or fragments. Operators must configure the
appropriate private network-policy peers for an external collector; enabling export
does not grant general worker network access. Leave both options unset to disable
export. Do not put prompts, provider credentials or repository content in exporter
configuration or attributes.

## Correlation and recovery

The API creates an admission span and freezes its validated W3C `traceparent` on
the command receipt. Immutable outbox bytes inherit that context when a turn is
eligible, including FIFO promotion after its predecessor finishes. Exact command
retries preserve the original context; diagnostic metadata does not participate in
command identity or grant authority. Broker publication, supervisor receipt,
session storage/Pod preparation, worker execution and report acceptance continue the
same trace. Model and tool spans use the retained turn context and their call or
invocation identity. Library and provider details remain behind allowlisted spans.

The web timeline exposes a retained conversation trace ID when one exists. The
inspector displays task attempt UUIDs as their compact 32-hex diagnostic trace IDs.
Task source preparation, coding, quality freeze/collection, application acceptance
and draft publication use that attempt trace. A confirmed recovery creates a new
attempt trace and links it to its explicit parent attempt. No operation leaves a
single span open for the lifetime of a project. Redelivery, retained-result replay and consumed-publication recovery create
bounded spans with causal links. Ordinary polling and first publication are not
classified as replay. Outbox replay is identified using retained
publication-attempt counts; broker redelivery uses the delivery count.

The credential-free managed-quality sandbox does not receive trace export
configuration. Trusted freeze/collection and acceptance operations bracket that
stage and correlate its retained outcome. Trace duration describes the measured
operation; accepted activity timestamps describe recorded observations. Neither
should be substituted for an unrecorded exact worker-stage start.

Span attributes contain bounded workspace/project/conversation/turn/plan/task,
attempt, session/generation and provider/tool identities where applicable. They
exclude prompts, source, tool arguments/results and raw upstream errors. These IDs
stay in traces and scoped history, rather than metric labels. Existing static-name
queue, broker, scheduling, provider and recovery metrics remain operational signals.
Admission and provider/task/publication authority handlers additionally expose
fixed-name request/failure counters and duration histograms through `/metrics`.

## Inspect and verify

After a real runtime journey, extract the collector's allowlisted correlation
projection and validate coverage:

```sh
python3 scripts/ci/check-execution-traces.py \
  --namespace ayni-orchestrator-acceptance-observable \
  --output /absolute/new/evidence/traces \
  --profile full
```

The checker uses only `kubectl --context docker-desktop` and the collector's
read-only reader container. It rejects evidence over 32 MiB and content-bearing
attributes. It writes `spans.json` and `outcome.json`; the output directory must be
new. Search `spans.json` for the inspector's trace ID to follow a journey without
requiring a UI vendor. `--profile conversation` checks a scoped conversation path.

The full profile additionally requires one task attempt across coding, managed
quality and publication, a repeated linked operation, and observed cancellation
reconciliation. It reports missing stages and separately counts model/tool spans:
scripted execution cannot establish a real provider call or token use. Pair these
diagnostics with the cumulative runtime/browser acceptance's retained interruption,
replay, quality and publication evidence. A connected trace alone does not prove
successful delivery, live-provider qualification or sustained capacity.

## Sources

- [Collector file exporter configuration](https://github.com/open-telemetry/opentelemetry-collector-contrib/blob/v0.147.0/exporter/fileexporter/README.md)
  defines the pinned file format and rotation behavior.
- The repository's [execution timeline](../architecture/execution-timeline.md) owns
  retained history; [operational acceptance](operational-acceptance.md) owns runtime
  qualification boundaries.
