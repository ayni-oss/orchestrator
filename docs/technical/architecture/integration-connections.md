# Integration connections

## Delivered foundation

The sole web application in `apps/web` uses the promoted example shell, shared
components, fonts and original light/dark tokens. It provides Workspaces and Configuration.
It creates and discovers workspaces/projects through canonical v1 APIs and manages
multiple named GitHub App source-control connections. Registry, OpenAI, repository catalog and associations remain later delivery;
placeholder pages are not exposed. The duplicate example and diagnostic frontend
are removed. All future journeys extend this application.

`ConnectionService` admits manual App credentials, changes enabled state, validates
saved candidates, lists connections/history and recovers a command by its original
idempotency key. Provider and capability are separate: GitHub currently implements
`source_control`. Each connection has a stable App ID; connect a different App as a
separate named connection.

## Storage and authority

PostgreSQL owns connections, immutable credential revisions, safe command receipts
and operation history. Credential values have no application-level encryption.
Hosting owns disk/database and backup encryption, database access control and
transport protection. Local development requires no encryption setup.

`workflow_connection_credentials` is separate from safe metadata. PUBLIC has no
access; the trusted runtime role can insert/read revisions but cannot update or
delete them. Workers have no database credentials. The current foundation has no
credential delivery to workers; OpenAI integration will implement the scoped
handoff and derived runtime mounts in a later PR.

Credentials are write-only through the product. Safe receipts contain the command's
non-secret fields and a credential revision reference. Exact credential retries
compare with protected storage, never with an extra plaintext copy or an unkeyed
credential hash in receipts. Browser storage retains only the opaque pending
connection command key. The credential field is cleared on submission, and an
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

Source access, registry validation, task execution and publication remain separate
authorities. No operation in this foundation creates GitHub branches, PRs or images.

## Qualification

The PostgreSQL gate exercises exact retries, restart reads, failed rotation,
disable/late-validation fencing, concurrent updates, safe history and immutable
credential privileges. The optional combined browser gate runs real PostgreSQL,
gRPC and the gateway; it loses a successful credential-write response and checks
reload recovery, field/storage redaction, negative key validation, concurrent-tab
conflicts and same-origin rejection. It uses synthetic credential material.

Live successful GitHub authentication requires a maintainer-provided App through
the UI and is reported separately from deterministic state-transition tests.
The current UI is a trusted local-operator surface, not a remote multi-user service.
