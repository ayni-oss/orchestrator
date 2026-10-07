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
The current SQLite schema is version 7. Recognized schema-6 databases migrate
additively at startup: one transaction creates certificate tables and advances the
schema marker, preserving existing resources, payloads and retained work. This
upgrade requires no database reset. Schemas 5 and earlier, unknown versions and
unrecognized table layouts are rejected; the earlier worker-owned breaking
cutover still requires reinitialization for those states. Retained-data deletion requires
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

Runtime registry selections are additive protobuf metadata retained by the
schema-7 upgrade;
this UI refinement requires no database reset. Existing profiles without a runtime
selection inherit their repository’s legacy default until explicitly edited.
After every runtime has an explicit selection, that legacy default no longer
retains an unused registry connection. Frozen attempts and pending builds continue
to retain their credential revisions. Changing a runtime destination is a
version-checked, idempotent mutation and wakes reconciliation without restarting
the control plane.

Runtime registry selection now stores only the connection ID. Older duplicated
namespace protobuf fields are reserved and ignored when existing payloads are
read; the schema-7 certificate migration preserves those payloads. Legacy
repository write APIs accept an omitted
namespace as inheritance, reject a supplied different namespace, and store the
connection's actual namespace as compatibility metadata. Runtime routing never
uses that legacy field. Reconciliation retires an old duplicated destination
before observing its frozen pending work; old Jobs finish at their original path
but cannot reactivate it. Newly admitted work waits for the normalized destination
to be verified.


## Certificate setup and rotation

Use **Certificates** to generate or import an Ed25519 signing key and its key ID.
The first valid certificate becomes the global default. Choose another enabled
default before disabling or deleting it; assigned certificates cannot be deleted.
Ayni runtime Edit dialogs can follow the default or explicitly select another
certificate. Projects inherit these repository runtime selections. Custom runtimes
receive no signing material.

Register administrator signing Secrets explicitly through
`connections.adminCertificates`. Legacy `build.signing_secret` references are
registered for compatibility and are deprecated for new setup. Updating a referenced
administrator Secret creates an owned immutable execution snapshot without changing
the source Secret. The UI displays public details and reports missing or invalid
sources. It never returns a private signing seed.

Selection changes, default changes and key rotation retire admission readiness and
propose draft public trust-policy/lock updates only for inspected Ayni repositories.
Review and merge the linked PR, then wait for image verification before admitting
new work. Closed PRs and failed lock jobs remain visible; use runtime Rebuild after
repair to retry. Source changes and stale builds cannot activate a newer selection.

Build the environment builder with `deploy/images/runtime-builder/Dockerfile`.
It includes Git, CA certificates, Docker CLI and Buildx, plus the pinned mise
resolver used by the supported Ayni CLI. Trust-lock jobs use this image without
Docker daemon access or signing and registry-push credentials. The resolver
version must match the repository lock; mismatches fail instead of silently
changing immutable provisioning inputs.

Retain previous immutable signing Secret revisions while pending or frozen build
references need them. Already admitted tasks retain their frozen runtime image;
rotation does not rewrite their snapshots. Never delete administrator-owned source
Secrets as certificate cleanup. The supported schema-6 to schema-7 upgrade keeps
existing connections and retained execution state; back up the private database
before upgrading and preserve Secret ownership references.
