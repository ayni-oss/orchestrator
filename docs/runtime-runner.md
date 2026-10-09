# Runtime worker

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
draft, with completion counts and bounded actionable findings.

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
dependency belongs to this private worker; it does not change Ayni upstream.


Environment preparation runs separate registry-discovery, build/certification,
publication and verification Jobs. Every successful build publishes. Coding workers
receive only the frozen verified digest, source/model access and pull-only registry
access. They have no signing key, push credential or engine/node socket. See
[managed runtime execution](managed-runtime-execution.md) for phase recovery.
