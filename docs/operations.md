# Operations

Runtime configuration sets `codingTurns` (default 24) and `qualityRepairLimit`
(default 3, accepted range 0–10). Admission freezes both limits in the task
snapshot. All phases share the frozen deadline. Worker admission requires the
pinned runtime digest and source identity; check failures are distinct from
unavailable infrastructure or evidence.

Task details display Progress, Quality and Pull request. Missing measurements
are unavailable. A published draft can have failed quality. Retry publication
resumes retained final work; retry task starts a new attempt with its frozen
snapshot. Persisted budgets apply after worker restart. Collection is idempotent
and resource cleanup follows durable collection.

## Breaking cutover

Use the explicit `docker-desktop` context for local deployment. Stop scheduling
and terminate old workers before reconnecting projects and repositories. Resolve
and display the exact application-owned database path and volume, output PVCs,
Jobs, ConfigMaps and attempt Secrets before deleting anything. Confirm ownership
from the release and workload references; never use a namespace-wide deletion.
The current SQLite schema is version 6. Reinitialize application state because
older schemas are rejected without conversion. Retained-data deletion requires
explicit approval after displaying the inventory. Preserve administrator-managed
connection Secrets and external GitHub Apps. Keep source repositories, runtime images, unrelated resources and
existing GitHub pull requests.

Remove orchestrator-installed quality workflows from connected repositories
through reviewed cleanup pull requests. Preserve environment-building workflows
and ordinary CI. Runtime credentials remain repository-scoped and are refreshed
while the worker is active. Never expose credential values in inventory or logs.

Recovery snapshots are private operational state and are never automatically
published. Preserve final work and its evidence until publication recovery has
finished. A conflicting remote head requires operator attention and must never
be overwritten.

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

## Connection credential retention

Credential rotation retains previous Secret revisions while unfinished Jobs or
frozen runtime references still need them. Terminal task snapshots also retain
their old pull Secret because task and publication retries use the frozen
snapshot. This deliberately favors retry recovery over immediate credential
cleanup; do not manually delete those Secrets while retained work can be retried.

Removing an administrator connection from deployment configuration disables it
without deleting its Secret. Connection deletion checks current, pending and
retired revisions for consumers before deleting UI-managed Secrets. Labels
identify ownership; Kubernetes RBAC grants Secret access by namespace and
resource, not by these labels.
