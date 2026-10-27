# Integration connections

## Configuration surface

The sole web application in `apps/web` uses the promoted example shell, shared
components, fonts and original light/dark tokens. It provides Workspaces and Configuration.
It creates and discovers workspaces/projects through canonical v1 APIs and manages
multiple named GitHub App source-control, GitHub Container Registry and OpenAI
connections. Workspace settings selects an OpenAI connection, discovered model,
instructions and budgets. Repository catalog and associations remain later delivery;
placeholder pages are not exposed. The duplicate example and diagnostic frontend
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
It does not yet exercise an artifact push/pull/cleanup probe or establish package
permissions; those recoverable external stages remain outstanding. Guided GitHub
App setup, installation/repository discovery and shared catalog associations also
remain outstanding.

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
