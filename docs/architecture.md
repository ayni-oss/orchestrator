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
and pull request receipts. SQLite schema version 8 retains certificate tables and
migrates recognized schema-6 databases transactionally without rewriting existing resources. Earlier
and unknown schema versions are rejected. Connectors, projects, repositories and runtime profiles remain
control-plane resources. Credentials are separate inputs and are never worker output.

## Connection credentials

GitHub App private keys and GHCR tokens live in namespace-scoped Kubernetes
Secrets. SQLite stores ownership, safe provider metadata, Secret references and
UIDs, active/candidate revisions and bounded test results. Credential writes are
never stored as generic idempotency request payloads or returned by read APIs.
The connection service is exposed only on the trusted internal listener used by
the operator BFF; the custom-client listener cannot manage credentials.

One configured GitHub App and multiple GHCR connections support two ownership
modes. Explicit deployment references register administrator-managed connections;
these remain read-only in the UI. UI-managed connections use labelled Secret
revisions whose ownership and UID are checked before mutation. Unlabelled Secrets
are never discovered or adopted automatically. Kubernetes RBAC is namespaced;
ownership labels do not restrict Secret access.

Connection reconciliation runs independently of runtime execution. The shared
GitHub credential provider serves both application operations and runtime
reconciliation, polling referenced revisions every 30 seconds. UI writes queue
validation and activation; no control-plane restart is required. A failed
replacement keeps the previous valid revision active and exposes its failure.
Missing or disabled active credentials block new authenticated operations.

GHCR authentication, push, pull and artifact cleanup run in scoped, digest-pinned
probe Jobs. Probe results are persisted before Job cleanup. Push credentials go
only to trusted builders and probes; candidate workloads receive a separate
pull-only Secret. Probe Jobs receive no Kubernetes API token, model or signing
credentials. Each logical runtime profile selects an approved connection. The connection alone
defines its image namespace;
runtime admission freezes the destination and credential revision for subsequent
work and retries. Runtime selections survive source sync and rebuilds. A destination
change creates a new selected revision and removes active admission authority.
Already admitted work retains its frozen destination and credentials; superseded
builds can finish but cannot activate the new selection. Secret retention follows
these frozen references, as described
in [operations](operations.md).


## Signing certificates

Global Ed25519 certificates are trusted-operator resources. The certificate
service and runtime certificate setter reject custom-client credential management.
Import requests carry write-only seeds, and generation uses operating-system
randomness. Private seeds and key IDs live in owned immutable Kubernetes Secret
revisions; SQLite and browser DTOs hold public keys, references, status and usage.
Explicit administrator Secret references become read-only certificates with owned
immutable execution snapshots; arbitrary Secrets are never discovered.

The first valid certificate becomes the enabled default. Ayni runtime profiles
follow that default unless an explicit certificate is selected. Custom runtimes
have no certificate selection. Missing or unusable certificates block new signed
builds while connection setup remains available. Selection, default and key changes
retire current readiness; pending observations cannot reactivate a superseded
runtime, and admitted tasks retain their frozen images.

Reconciliation inspects the connected default revision before preparing public
trust changes. A scoped Kubernetes job validates the existing Ayni configuration
and regenerates the lock with the pinned CLI while preserving immutable inputs.
It receives no signing or registry-push material. Durable generation identities
and repository branches deduplicate draft trust-update PRs. After merge,
reconciliation checks the committed policy and lock, resolves or rebuilds the
signed image, and verifies the selected certificate before restoring readiness.
Only trusted environment builders receive signing seeds.


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

For a connection whose namespace matches the source owner, the computed runtime
destination is `ghcr.io/<namespace>/<repository>`. Cross-owner destinations
retain `ghcr.io/<namespace>/<source-owner>/<repository>` so repository names
remain distinct. One backend helper derives build, pull, admission and read-only
UI destinations. Deployment-only fallback settings retain their existing layout.
Previously admitted tasks and pending Jobs retain their original frozen paths.


## Repository image lifecycle

Runtime preparation has one implementation: persisted control-plane intent,
pull-only registry discovery, isolated build/certification, automatic publication,
and a separate pull-only verification Job. A verified manifest digest, selected
runtime revision and source identity gate admission. A later selection cannot be
activated by a superseded Job. Tasks retain their frozen image digests.

Discovery uses `ayni-<arch>-<full-lock-hash>` on the approved repository path.
Source owners are retained beneath a different registry namespace. A tag is a
lookup key; only independently verified immutable manifest digests authorize work.
[Managed runtime execution](managed-runtime-execution.md) defines recovery and
credential boundaries. No GitHub workflow installation participates in readiness.
