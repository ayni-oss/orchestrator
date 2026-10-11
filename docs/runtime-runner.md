# Runtime worker

This is the implemented worker contract. Optional independent assessment and separate task-stage pods remain
[execution-isolation research](https://github.com/ayni-oss/orchestrator/issues/128).
Ordinary lock protections remain until the [environment-change flow](environment-changes.md) is implemented.

One worker owns runtime admission, checkout, coding, commits, full quality
checking, repair, push and draft creation. Kubernetes fixes the image digest,
resources, output isolation and deadline. Repository-scoped credentials are
mounted separately for checkout and delivery.

`ayni_verify` directly invokes `ayni verify <signal>` with typed supported
selectors and reads `.ayni/verify/last/signals.json`. `ayni_check` directly invokes
`ayni check` and reads `.ayni/last/signals.json`. Focused evidence never satisfies
the full quality gate. The coordinator always runs the full check even if the
agent calls no verification tool.

Coding sessions invoke a `CodingAgentRunner` adapter. The current adapter uses
Rig and OpenAI; it owns model credentials, tools, usage tracking and the hook
that persists consumed turns before provider calls. Session state, recovery and
deadline calculations stay outside that adapter. Replacing the adapter must
preserve the frozen budget, bounded conversation and input-question result.
Ayni's supported CLI currently supplies environment and quality execution; an
external Ayni coding-agent invocation contract is not yet implemented here.

Coding turns are counted across initial and repair sessions. Coherent milestones
use Conventional Commit messages, and remaining permitted changes are committed
when a session ends. Empty commits are skipped. Repairs retain original context
and bounded conversation. Operational failures receive bounded retries rather
than being represented as source findings.

Before delivery, freeze the full artifact, digest, source fingerprint, summary
and final commit. A check that modifies publishable files requires another commit
and full check. Failed or unavailable checks are disclosed truthfully in the
draft, with completion counts and bounded actionable findings. The final check uses
`ayni check --output markdown`; its generated report is bound to the same artifact
digest and final commit, then retained outside the sandbox. After opening the draft,
the worker posts that Markdown as an Ayni quality comment. Publication retries
reuse the PR receipt and recover an uncertain comment response by matching the
exact report, so they do not create another PR or duplicate its comment. Reports
above GitHub's comment limit are explicitly truncated; the full report is retained.

The task Quality section uses Ayni's pass, warning and failure icons. Hovering or
focusing a signal reveals its generated summary and language/root. Legacy attempts
retain their recorded pass/fail icons; summaries and report comments require a
new final check and are not reconstructed from the agent's narrative.

Atomic recovery preserves workspace changes, local commits, phase and budgets.
Once publication starts, recovery resumes delivery without recoding. Persisted
intent identifies the deterministic attempt branch; persisted receipts identify
the adopted or created draft and precise head.

## Worker sandbox

The Linux worker requires fully enforced Landlock ABI 6 (kernel 6.12 or newer).
Both agent shell commands and Ayni quality execution run inside the same
unprivileged sandbox because repository tests are untrusted code. System and
toolchain paths are read-only; the workspace and disposable scratch paths are
writable. Worker output, repository/model credentials, frozen requests and the
launcher remain outside that authority. Signal and abstract Unix-socket scopes
prevent sandboxed children from signaling the parent or reaching its abstract
sockets. Unsupported or partial enforcement fails closed: there is no
unrestricted fallback. This needs no privileged container, added capabilities,
or special Docker/Kubernetes privilege.

The coordinator binds and retains final evidence outside the sandbox. Agent and
repository processes cannot rewrite worker progress, publication intent, final
evidence or receipts in the retained output directory. The Linux-only Landlock
dependency belongs to this worker; it does not change Ayni upstream.


Environment preparation runs separate registry-discovery, build/certification,
publication and verification Jobs. Every successful build publishes. Coding workers
receive only the frozen verified digest, source/model access and pull-only registry
access. They have no signing key, push credential or engine/node socket. See
[managed runtime execution](managed-runtime-execution.md) for phase recovery.

## Retained runtime verification

New verification Jobs select the `retained-v1` reporting contract in their Job
annotation. The trusted launcher writes `verification.json` atomically in the
Job's subdirectory of the existing runtime-output claim. The record is bounded
to 128 KiB and binds the Job name, physical Pod UID and exact frozen-request
bytes by SHA-256. It contains the complete `VerificationReport` or a safe typed
failure classification; raw exceptions and command output are not retained.
Repository children cannot access `/output`, and checkout does not mount it.

The controller validates the owned Pod and request binding before interpreting
the report, independently of Job success or process exit. Existing runtime
certificate, source, lock and digest admission checks still apply. A failed
source-quality check remains a quality outcome rather than a runtime-trust
failure. Admitted evidence is persisted before resource cleanup. Retained files
survive Job deletion; uncollected files without Kubernetes identity cannot grant
readiness. They remain available for diagnostics in the claim after restart.

Termination messages carry only a compact reference for new Jobs. Jobs without
the reporting annotation keep their legacy termination-message reader; unknown
contract versions fail closed. A record cannot be overwritten by a later Pod.
Reporting, missing evidence, invalid identity and oversized records have separate
reason codes.

Runtime preparation records failed build, verification, publication and trust Jobs by their
persisted execution name. Reobserving a terminal Job after restart counts it once.
Build and verification retries wait for owned-resource cleanup before admitting a
fresh Job identity; three failed executions exhaust the preparation budget. Trust
failures require operator retry. Publication records outcomes while retaining its independent bounded
attempt/readback protocol and requires retry when that budget is exhausted.
Observation, evidence collection and cleanup errors have separate backoff counters
and preserve the execution intent without spending this budget. Missing or
unavailable reports remain reporting errors, never inferred verification failures.

The profile exposes retry eligibility, failure class and recorded execution count.
Exhaustion hides the automatic retry timestamp. Sync becomes Retry when attention
is required; it uses the existing durable reconciliation action. An admission key
is retained before acknowledgement so crash replay cannot replenish a used budget.
Retry preserves failure history and does not authorize replacing a runtime image;
Rebuild remains the explicit replacement action. Transient reporting retries reobserve the same retained evidence. Explicit Retry
admits a fresh execution after missing or invalid terminal evidence; it first
cleans the old owned workload. Repository analysis leaves preparation budgets unchanged;
changed frozen source, policy, lock or infrastructure identity admits a new
preparation. Legacy observation counters remain conservative budget values and
are shown separately as legacy failures, without inventing execution history.
