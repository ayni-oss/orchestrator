# Architecture

The control plane admits tasks, freezes repository and runtime identity, model,
constraints, total coding turns, quality repair limit and deadline. It schedules
one worker, supplies repository-scoped credentials, observes durable progress and
results, and handles cancellation and cleanup. It never pushes task Git changes
or opens task pull requests.

The worker attests the runtime before checkout, verifies the source and managed
lock, codes with focused `ayni_verify` feedback, commits coherent milestones,
and always runs `ayni_check` against the full repository. Actionable failures can
start bounded repair sessions within the same total turn budget and deadline.
The final evidence must bind the checked source and final commit. Missing or
malformed evidence is unavailable and cannot pass.

Execution, quality and publication are independent outcomes. Valid changed work
can become a draft with failed or unavailable quality. Admission failures,
unsafe changes, cancellation and no-change outcomes do not publish. The worker
persists publication intent before push and a receipt before completion. It
adopts a matching draft after uncertain provider responses and stops on a
conflicting remote head.

Atomic recovery generations retain changes, local history, phase and consumed
budgets without adding Git commits. Recovery after publication begins resumes
publication. Retry publication uses retained final work without another coding
session; ordinary task retry creates a new attempt.

The API exposes frozen task snapshots and attempt-owned worker progress, results
and pull request receipts. SQLite rejects an older schema rather than converting
it. Connectors, projects, repositories and runtime profiles remain control-plane
resources. Credentials are separate mounted inputs and are never worker output.

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
