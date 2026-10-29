# Integration connections

## Configuration surface

The sole web application in `apps/web` uses the promoted example shell, shared
components, fonts and original light/dark tokens. It provides Workspaces and Configuration.
It creates and discovers workspaces/projects through canonical v1 APIs and manages
multiple named GitHub App source-control, GitHub Container Registry and OpenAI
connections. Workspace settings selects an OpenAI connection, discovered model,
instructions and budgets. Repositories provides shared source discovery and catalog admission. Project cards link to their repository associations. Runtime preparation remains later delivery. The duplicate example and diagnostic frontend
are removed. All future journeys extend this application.

`ConnectionService` admits manual App credentials, changes enabled state, validates
saved candidates, lists connections/history and recovers a command by its original
idempotency key. Provider and capability are separate: GitHub currently implements
`source_control` and `container_registry`; OpenAI implements `llm`. Each connection has a stable App ID; connect a different App as a
separate named connection.

## Storage and authority

PostgreSQL owns connections, immutable credential revisions, safe command receipts
and operation history. Credential values have no application-level encryption.
Hosting owns disk/database and backup encryption, database access control and
transport protection. Local development requires no encryption setup.

`workflow_connection_credentials` is separate from safe metadata. PUBLIC has no
access; the trusted runtime role can insert/read revisions but cannot update or
delete them. Workers have no database credentials. Selected OpenAI turns freeze the active credential revision at admission under the
connection lock. Disabling blocks new admissions without changing retained work.
Credential-bearing consumers use a fresh session generation per turn, including
credential changes. Late terminal cleanup therefore cannot delete a later turn's
consumer or mount. The
supervisor authenticates to the internal `/v1/session-credential` endpoint with the
exact accepted, claimed command. Only that owned session's frozen credential is
returned; this endpoint is absent from public RPC and browser allowlists. The
supervisor creates an immutable, session-labelled Secret and mounts it read-only
in the worker. The worker has no database credentials. Terminal acknowledgement
removes the consumer Pod before deleting its ownership-checked derived Secret;
the retained route resumes cleanup after interruption. PostgreSQL remains authoritative.

Credentials are write-only through the product. Safe receipts contain the command's
non-secret fields and a credential revision reference. Exact credential retries
compare with protected storage, never with an extra plaintext copy or an unkeyed
credential hash in receipts. Browser storage retains only the opaque pending
connection command key. Workspace configuration retries retain non-secret command
inputs and their idempotency identity. The credential field is cleared on submission, and an
ambiguous response is recovered by key before another submission. Explicitly
abandoning recovery permits a new command and may leave the original candidate
retained; it never removes accepted history.

## Validation and rotation

Saving credentials admits an operation and candidate atomically; it does not claim
access success. A separate explicit validation resumes that operation, signs an App
JWT and makes a bounded, non-redirecting read to GitHub's fixed `/app` endpoint.
It verifies the observed App ID and retains only a closed safe reason vocabulary.
It does not establish repository installation access or runtime readiness.

Acceptance checks the candidate, connection revision and enabled state under a row
lock. A superseded or disabled generation cannot activate. Successful validation
promotes the candidate; a rejected candidate preserves the previous active revision.
An outage or rate limit leaves the operation resumable with its original identity.
Concurrent validation reads are harmless; accepted terminal outcomes are stable.
Enabling/disabling uses exact revisions, preventing stale tabs from overwriting
current configuration. Re-enabling does not validate or promote a candidate.

OpenAI validation uses the standard `/v1/models` endpoint, retaining only bounded
model identifiers. No paid conversation is sent by validation. Model discovery does
not establish compatibility with the conversation execution API. Custom endpoints
are not accepted by the configuration UI. Selected connection sessions always use
the standard OpenAI endpoint; existing isolated provider fixtures remain separate.

GHCR configuration requires an explicit `ghcr.io/owner/package` destination and
separate push/pull usernames and classic PAT inputs. Destination and account
identities are immutable; create another named connection to change them. Candidate
validation independently authenticates both credentials using scoped token requests.
It does not establish package permissions. Runtime operations must report authentication,
push and pull failures at their actual stage; no disposable test image is required. Guided GitHub App setup remains outstanding. Saved source connections support installation/repository discovery and shared project associations through the repository catalog.

Source access, registry validation, task execution and publication remain separate
authorities. No operation in this foundation creates GitHub branches, PRs or images.

## Qualification

The PostgreSQL gate exercises exact retries, restart reads, failed rotation,
disable/late-validation fencing, concurrent updates, safe history and immutable
credential privileges. The optional combined browser gate runs real PostgreSQL,
gRPC and the gateway; it loses a successful credential-write response and checks
reload recovery, field/storage redaction, negative key validation, concurrent-tab
conflicts, GHCR/OpenAI save/reload recovery, workspace connection/model selection
and same-origin rejection. It uses synthetic credential material.

Live successful GitHub, GHCR and OpenAI evidence requires maintainer-provided
credentials through the UI and is reported separately from deterministic fixtures.
The current UI is a trusted local-operator surface, not a remote multi-user service.

## Shared repository catalog

Open **Repositories**, choose a validated source connection, and discover its GitHub
installations. Select an installation, discover its repositories, then connect one.
Projects attach catalog entries directly through their **Repositories** action;
projects in several workspaces can use the same entry.

`RepositoryService` provides bounded v1 discovery, connect/recovery/history,
catalog reads and project associations. PostgreSQL migration 32 stores immutable
provider repository identity, the source connection and installation, connect intent,
accepted metadata and exact association receipts. Catalog records are append-only;
removal and source rebinding require a later explicit lifecycle contract.

GitHub requests use the fixed API origin, bounded response bodies and timeouts.
The saved App credential creates read-only installation tokens with Contents and
Metadata read permissions. Connecting restricts that token to the selected repository
ID and checks GitHub's returned identity; browser names and branch values grant no authority.
Tokens and provider response text never enter catalog receipts or diagnostics.

Connect intent freezes the connection revision and credential reference before
external reads. Disabling or rotating that source fences acceptance; an outage or
rate limit retains a resumable operation. Exact retries recover one receipt,
including after a lost response or process restart. History includes safe reasons,
source-access stage and accepted/update times; structured logs retain operation IDs.

An existing provider repository cannot silently move to another source connection.
Association commands validate workspace/project ownership, reject disabled sources
for new use, and retain exact retry receipts. Associations expose no credentials or
conversation history and confer no execution or publication grant.

**Connected** means source access was observed at connection time. It does not
establish current availability, committed Ayni compatibility or runtime readiness.
Task source verification and runtime artifact selection remain separate future work.

`AYNI_REPOSITORIES_ONLY=1 scripts/ci/check-workflow-postgres-feasibility.sh`
checks real PostgreSQL, gRPC, gateway and browser recovery using synthetic source
credentials. It qualifies catalog authority and UI behavior, not live GitHub
credentials, registry permissions or runtime preparation.

## Frozen static Ayni inspection

`InspectRepository` accepts an existing catalog ID, the displayed source connection
revision and a full lowercase commit SHA. It records intent before GitHub reads;
repeating the exact request recovers the same operation. History and recovery reads
never resume external access. Source rotation or disablement fences late results.

The installation token is scoped to that repository with read-only Contents and
Metadata permissions. Commit and tree identities are checked; required blobs come
from that frozen tree, with their Git object identity and size independently
verified. Policy and lock bytes produce retained SHA-256 digests. Repository files
are never executed, and raw provider errors and source bytes are not returned.

Static evaluation consumes `ayni-core` and `ayni-environment` from Ayni 0.16.2 at
`8eb4c7c8abcbd7ecb4bbda26e0140b58ffcfc6dc`. These public APIs validate policy,
lock schema/recipe/fingerprint, locked input digests, host architecture and public
trust fingerprint consistency. This fallback is needed because the CLI has no
standalone read-only lock inspection command. It adds no dependency from Ayni to
Orchestrator and accesses no private signing material.

| Limit | Behavior |
| --- | --- |
| Source selection | Full 40-character lowercase commit SHA; branches and tags rejected |
| GitHub responses | Bounded to 1 MiB each; truncated trees rejected |
| Snapshot | At most 256 required regular files, 512 KiB each, 8 MiB total |
| File paths | Repository relative; symlinks, submodules and escaping paths rejected |
| Provider work | 25-second request deadline; transient provider/local unavailability and rate limits retain the same retry identity |
| Static result | Bounded to 64 KiB; retained independently of browser lifetime |

A completed inspection can contain blocking findings. Missing or invalid inputs
produce corrective findings rather than runtime readiness. Every result retains
pending adapter-discovery and runtime-verification requirements. Registry setup,
selected signer admission, trust-change PRs, image preparation and task handoff
remain later slices. Reading or inspecting a repository never opts it into builds.

The repository browser gate covers retained result display, exact commit input,
response loss/reload recovery and conflicting retries against real PostgreSQL,
gRPC and the gateway. It uses synthetic GitHub credentials and separately seeded
static findings; it does not establish live GitHub source qualification.

## Runtime preparation foundations

Runtime preparation currently provides public signer-reference configuration, an immutable
GHCR discovery client and a trusted content-verifier binary. These are preparation
foundations; the build/sign/publication supervisor and its UI journey are not
implemented. No runtime can be admitted as ready through these primitives alone.

Runtime signing configuration stores a reviewed key ID/public key and the name
and exact UID of an administrator-owned immutable Kubernetes Secret. The namespace
belongs to deployment configuration. The product does not read the referenced
object when saving metadata. Preparation must later validate immutability, UID,
private/public key agreement and stage authority before delivering signing access.

Signer changes use exact command receipts and immutable public revisions. Rotation
and enable/disable operations fence stale versions. Read APIs and lost-response
recovery return public configuration only. There is no signing material or signer
credential reader in PostgreSQL. Initial administrator provisioning remains an
external prerequisite, with the reserved `ayni-runtime-signer-` object-name prefix.

The registry library distinguishes confirmed manifest absence from access failure,
rate limiting, outage, malformed data and unsupported platform. Manifest and config
bytes must match their immutable digests. Discovery returns a candidate, never
verification or replacement authority. Registry publication is not implemented.

`backend/workers/runtime-verifier` consumes frozen policy/lock inputs, a platform
and an independently trusted expected Ayni executable digest. It invokes the
pinned Ayni public verifier against the actual candidate root, including signature
and protected content, without executing the candidate's CLI. Its `content_verified`
result covers certification/content only. Full adapter discovery, managed execution,
operation-generation binding, cleanup and runtime admission remain separate gates.
The packaging Dockerfile rejects a dynamic interpreter or shared-library dependency
before producing the trusted verifier image.
