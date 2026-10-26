# Scoped MCP integrations

> UI status: the console and initial product workspace are removed. Browser steps
> and evidence below describe the retired UI and are historical reference only.
> Browser-dependent acceptance profiles are unavailable until replacement coverage
> is implemented. The current local API adapter is `dev/ctl gateway`; it serves
> no UI. See [the retained UI example](../../../examples/web/EXAMPLE.md).

The isolated conversation worker supports configured MCP tools through a bounded
JSON-response Streamable HTTP profile. Configuration is operator-owned deployment
input; model text cannot add servers, choose credential paths or expand scope.

Enable `ORCHESTRATOR_MCP_ENABLED=true` on the session supervisor. It mounts the
workspace ConfigMap `mcp-config-<workspace UUID without hyphens>` at
`/var/run/mcp-config/grants.json`, plus an optional workspace Secret
`mcp-credentials-<workspace UUID without hyphens>` directly at `/var/run/mcp`.
The worker receives credentials through that volume; credentials never enter
configuration, transcripts, tool results or evidence projections.

The ConfigMap's `grants.json` contains an array of grants. Each selects an exact
`workspace_id`, `project_id`, and `configuration_revision_id`, with at most five
tools. A tool has `alias` (a bounded `mcp_` identifier), remote `name`, HTTPS
`endpoint`, `description`, `input_schema`, `read_only`, `max_output_bytes`
(1–32768), `timeout_seconds` (1–30), nullable `bearer_file`, and nullable `consent`.
HTTP is accepted only for loopback fixtures. Credential paths are restricted to
one filename below `/var/run/mcp/`. Redirects, query strings and credential-bearing URLs fail before configuration
can enter the retained tool descriptors. Use the directly mounted bearer file
for authentication.

Preparation freezes the selected complete tool descriptors in the session ledger
before any provider request. An already retained preparation reuses its original
allowlist even after deployment configuration changes. Tool schemas and outputs
consume the existing cumulative provider budget; at most eight invocations and
eight rounds are permitted per turn. No tool is discovered or enabled implicitly.
The supported schema subset is bounded objects, arrays, primitive types, required
properties, enums and descriptions; unsupported constraints fail configuration.
Exact schemas validate every invocation before I/O.

## External-effect consent

Read-only tools need no mutation grant. Every other tool requires an operator
consent object with exact `command_id`, `arguments_digest`, and
`acknowledge_external_effect: true`. The alias belongs to the same frozen tool
descriptor. `arguments_digest` is SHA-256 of the parsed arguments serialized as
compact JSON with sorted object keys, including the `sha256:` prefix. It does not
hash whitespace in the provider's argument string. The command is the exact
retained conversation command ID, not the turn text or provider response ID.

This is an exact operator authorization surface, with no wildcard scopes or
self-approval by the model. Provision the consent before the command's first
preparation; editing the ConfigMap cannot change an already retained preparation.
Inspect the intended command and arguments before granting consent. Console-based
interactive approval is not provided by this deployment-level surface.

Before external I/O, the worker durably consumes the command/alias/argument grant
and records `sending`. The same invocation ID replays only its retained result;
a different ID cannot consume the grant again. A lost response, cancellation or
restart with `sending` retains uncertainty and never automatically repeats the
external action. Explicitly reconcile the external system before authorizing a
new command. Cancellation stops local waiting; it does not prove remote rollback.

## Protocol and acceptance boundary

Each invocation initializes a separate MCP session, validates its tools capability,
sends the initialized notification and reads one bounded `tools/list` page. The
requested name and schema must match the frozen descriptor before `tools/call`;
discovery cannot add tools or expand scope. Tools absent from that page fail closed. Session IDs returned at initialization
are carried on subsequent requests. Initialization and tool responses are bounded
before parsing, and only normalized tool results reach the model. The profile
supports protocol `2025-03-26` JSON responses; SSE, server-initiated requests,
resumption and legacy transports fail closed. This is a limited profile of the
[MCP Streamable HTTP transport](https://modelcontextprotocol.io/specification/2025-03-26/basic/transports).

Run `dev/fixtures/mcp/acceptance.sh RUNTIME_IMAGE` against explicit Docker Desktop
Kubernetes for the owned session-worker/provider/MCP fixture. It retains a PVC
through Pod replacement and checks that accepted read results and the one
consented external action replay without another provider or MCP call. Public
fixture credentials have no external capability. The script removes only its
owned namespace on completion. Pass `anthropic` as the second argument to exercise
the selected Messages provider, MCP continuation, transport uncertainty, in-flight
cancellation and replay of accepted and failed calls after Pod replacement.

Deterministic unit tests cover scoped schema validation, unsafe endpoints, mutation
consent, exact replay, consumed-grant rejection, initialization and output limits.
Unit tests alone do not establish deployed failure acceptance; the separate local
profile above exercises that topology. Neither establishes live external-service
interoperability. The delivery PR records executed evidence and remaining limitations.

## Stage exact mutation consent

Use the trusted local operator workflow to bind consent before the session prepares
a request. Pause the local `conversation-supervisor` StatefulSet, admit the intended
message through the console, and read `GetTurnInputs` for its complete workspace,
project, conversation and turn scope. Its `command_event_id` is the exact retained
execution identity; the admission receipt's `command_id` identifies a different
operation. An empty event identity means the turn is still queued behind earlier
work and cannot yet receive an execution-bound grant.

Install the workspace MCP manifest with the returned frozen configuration revision,
that exact event identity, tool alias and canonical JSON argument digest. Mutation
consent must acknowledge the possible external effect. Resume the executor only
after reviewing the manifest. The configured endpoint, input schema, limits and
consent freeze when preparation commits; editing the ConfigMap afterward cannot
change a retained request or grant another invocation.

This procedure pauses local conversation dispatch for the release. It belongs to
the trusted deployment surface and requires operator access; ordinary model text
cannot stage consent or select another turn. Read-only grants do not require this
staging step. The session rejects ungranted effects, and uncertain effects require
external reconciliation before the operator authorizes any later action.
