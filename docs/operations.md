# Local operations

> **Clean-v1 runbook.** The versioned clean-v1 database, durable resources,
> idempotency, resumable observation, and shared lifecycle reconciliation are
> implemented. The pilot's preparation lifecycle, retained archives,
> `pilot_ready`, and additive migration instructions are retired. Provider
> automation, runtime execution, and managed assessment are delivered by their
> respective follow-on issues.

The supported topology remains one trusted maintainer team on Docker Desktop
Kubernetes, explicit `docker-desktop` context, and loopback-only browser/gRPC
access. It is not a public, multi-tenant, multi-node, or production service.

## Readiness and observability

Controller health is distinct from a repository profile. Readiness exposes a
healthy controller even while a profile is `setup_required`, `setup_pr_open`,
`awaiting_environment`, `updating`, or `degraded`. It must not claim a profile
is ready merely because a provider catalog, mutable tag, or process-local flag
is available.

Metrics and structured logs use bounded, low-cardinality state/outcome labels
for profile transitions, reconciliation duration/outcome, GitHub rate limits,
workflow wait, GHCR verification/drift, image pull, coding, assessment import,
comment update, and readiness action. They never use repository, commit, digest,
workflow, task, PR, raw signal, URL, credential, or log text as metric labels.
Exact identities live only in the bounded API/UI, durable records, and retained
evidence where authorized.

## Ayni Managed operation

1. Confirm the GitHub App installation and repository connection. Missing
   Contents, Pull requests, Actions, Checks, Packages, or comment capability is
   reported individually; do not broaden credentials as a workaround.
2. Create/reconcile the owned pipelines setup PR. A closed/unmerged or drifted
   PR leaves the profile non-ready. Repair the installation or owned files, then
   reconcile the durable setup action; never edit the default branch directly.
3. After merge, observe the trusted default-branch workflow. It validates the
   current `.ayni.toml`/`.ayni.lock`, Ayni release, supported platform, and
   workflow-template contract, then publishes or verifies only the expected
   enrolled-repository GHCR package/tag.
4. The controller resolves `lock-<full-64-hex-lock-fingerprint>` and verifies
   package ownership, OCI digest, platform, Ayni labels/schema, and bounded
   metadata. Only then can a profile be `ready`.
5. Coding tasks freeze profile revision and runtime `image@sha256:...`. A
   digest pull on another node is valid; building in the coding Job is not.
   Agent host checks are advisory.
6. A draft PR is assessed only by an exact-head managed workflow. Retain the
   immutable evidence/manifest and verify declared digests before importing the
   assessment. GitHub check/comment are projections and cannot authorize
   readiness by themselves.

Source-only default-branch changes must verify/reuse the existing valid artifact
without build/push. A valid lock change creates a new profile revision in
`updating`; old attempts may finish with their frozen digest, but new tasks wait
for the replacement. Existing lock tags are never silently moved.

## Recovery matrix

| Symptom | Required safe action |
| --- | --- |
| Setup PR drifted, closed, or not merged | Keep onboarding non-ready, reconcile the owned setup action after repair, and do not mutate default branch directly. |
| Lock missing, stale, malformed, or unsupported | Degrade/update that profile revision and block new task admission until a merged valid lock and verified artifact exist. |
| Workflow failed, timed out, or reports stale source/head | Retain run/attempt identity, do not import success, and reconcile exact current provider state before an allowed retry. |
| GHCR tag absent, moved, foreign, wrong-platform, or bad metadata | Mark artifact/profile degraded; preserve historical frozen digests; require fresh trusted publication and verification. |
| GitHub/GHCR credentials unavailable or revoked | Persist the failed action and bounded remediation, then resume reconciliation after access returns without duplicate mutations. |
| Candidate PR head changed or evidence is corrupt/oversize | Retain but reject the result as current assessment; do not update readiness or replace trusted evidence. |
| Controller/pod restart or uncertain GitHub response | Resume durable actions, read provider/registry state before retry, and converge without duplicate PR/comment/action effects. |
| Coding pull/runtime unsupported | Return an actionable artifact/platform result, clean task-scoped pull credentials, and never fall back to a mutable tag or local build. |

## Data, backup, and cleanup

The clean v1 database retains protobuf resource payloads and indexed identities,
monotonic versions and event cursors, durable actions, reconciliation cursors,
and bounded pagination snapshots. Startup validates the clean schema marker and
all persisted protobuf payloads before serving. It fails closed on an absent or
unsupported marker in a non-empty database, any retired pilot table, or corrupt
payload; it does not partially migrate, delete, or serve that state.

For the one-time local reset, stop writers and archive any pilot volume that must
be retained outside Kubernetes, then run `dev/ctl purge <exact-namespace>`,
`dev/ctl secrets`, `dev/ctl deploy`, and `dev/ctl check` as specified in
[local development](local-development.md#one-time-pilot-reset). Purge deletes the
whole labeled local namespace, including Secrets and retained volumes. Never
restore a pilot database into v1 or bypass rejection by editing schema metadata.

For clean-v1 backup and restore, quiesce the controller and preserve the database
and any referenced evidence as one consistency unit. Restore only to the same
supported clean schema version. After restore, verify readiness and resume
reconciliation. Durable external actions retain their bounded canonical request
payload in pending/completed/failed state. Reconcilers enumerate pending intent;
a failed action is requeued only after reading provider state, so an uncertain
response cannot cause a duplicate effect.

List continuations use signed opaque snapshot tokens, expire after 15 minutes,
and are capped by item, byte, response, and concurrent-snapshot limits. Watch
history is bounded per service. A cursor older than retained history—including
an explicit zero after pruning—or ahead of the service high-water mark returns
`OUT_OF_RANGE`/`WATCH_CURSOR_EXPIRED`; callers then take a current snapshot and
resume from its cursor rather than guessing an offset.

Retained evidence is addressed under its assessment and digest. Backup policy
must preserve the database and evidence together; a database-only restore that
loses referenced evidence fails closed rather than synthesizing an assessment.
#67 defines safe artifact retirement: never delete an active, in-flight,
open-change-request, assessment/evidence, held, unknown, or shared-digest
artifact; deletion is explicit and dry-run first.

Keep operational access loopback-only. Kubernetes Secrets, GitHub App keys,
registry/model credentials, raw workflow logs, and artifact bytes are never
placed in comments, metrics, API browser DTOs, or retained diagnostic text.
See the [GitHub and GHCR security boundary](github-security.md) for required App
permissions and workflow rules, [Decision 0007](decisions/0007-clean-v1-runtime-profiles.md)
for the normative trust and identity model, and #68 for the reproducible
end-to-end gate.
