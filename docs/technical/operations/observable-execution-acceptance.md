# Observable execution acceptance

Run this profile from a clean, committed checkout on Docker Desktop Kubernetes.
It combines retained task delivery and recovery with browser progress, inspector
reads and locally exported correlated traces. It does not require a live provider
credential or publish to GitHub.

## Prepare and run

```sh
pnpm install --frozen-lockfile
pnpm exec playwright install chromium
dev/ctl quality-runtime
AYNI_ORCHESTRATOR_NAMESPACE=ayni-orchestrator-acceptance-observable \
  AYNI_ORCHESTRATOR_TASK_FIXTURE=true \
  AYNI_ORCHESTRATOR_OBSERVABILITY=true dev/ctl deploy
AYNI_ORCHESTRATOR_NAMESPACE=ayni-orchestrator-acceptance-observable \
  dev/ctl acceptance /absolute/new/observable-evidence --observable
```

Both runtime images must be built from the current clean commit. The profile
checks their source labels and refuses to overwrite evidence. The collector must
be enabled before execution. See [execution tracing](execution-tracing.md) for
collector configuration, export limits and diagnostics.

The task phase runs the existing publication profile, including isolated coding,
managed Ayni quality, cancellation, Pod loss, publication reconciliation and
retained-store restarts. Publication uses real Git against the local GitHub
protocol fixture. These are deterministic coding fixtures; this does not qualify
autonomous coding or live GitHub authentication.

The browser phase uses the deployed loopback gateway and actual PostgreSQL,
JetStream and isolated sessions. It observes provisional scripted text before
terminal acceptance, reloads during generation, checks failed-progress-read
recovery and exact interrupted-command retry, then inspects the retained task's
coding, quality, publication, timing and recovery evidence through canonical APIs.
The scripted streaming journey begins with `Observe streaming` and intentionally
holds three bounded partial observations for one second each. Ordinary scripted
execution retains its normal timing.

The trace phase verifies correlated API, outbox, broker, supervisor, Pod and worker
spans for a conversation, plus source, coding, quality and publication for a task.
Repeated operation links and cancellation observations supplement the authoritative
replay and fencing evidence from the cumulative task profile. Traces are diagnostic
samples and never establish accepted work or exact accounting.

## Evidence and limits

The output includes the source revision and manifest, task-profile evidence,
browser screenshots and outcome report, local OTLP exports, trace assertions and
final readiness. Every failure leaves `outcome.json` marked failed. The namespace,
PVCs and accepted work remain retained; only the profile's gateway and local
port-forward processes stop automatically. Collector data is bounded diagnostic
storage and can be lost when its Pod is replaced.

OpenAI and Anthropic streaming adapters are exercised separately against loopback
HTTP SSE fixtures. The deployed browser profile uses the scripted provider and
makes no paid model calls. Live provider qualification and sustained production
capacity remain separate [operational acceptance](operational-acceptance.md)
requirements. Browser fixtures alone do not establish deployed behavior.
