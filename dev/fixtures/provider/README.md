# Deterministic provider fixture

This server implements the bounded foreground Responses profile without calling
OpenAI. It supports bounded streaming Responses and Anthropic Messages SSE
profiles, as well as complete JSON fixtures. Only public assistant text is emitted
as preview; tool arguments remain provider protocol data. It uses the public dummy credential `provider-fixture-key` and model
`fixture-model`. It rejects provider state, native tools and unsupported request
profiles. It retains request counters and client call IDs; it does not retain
prompts, response text or authorization headers.

Deploy only to the local Docker Desktop context, with the same namespace as the
control plane. For example:

```sh
kubectl --context docker-desktop -n ayni-orchestrator-dev create configmap provider-fixture \
  --from-file=server.mjs=dev/fixtures/provider/server.mjs --dry-run=client -o yaml \
  | kubectl --context docker-desktop -n ayni-orchestrator-dev apply -f -
kubectl --context docker-desktop -n ayni-orchestrator-dev apply -f dev/fixtures/provider/workload.yaml
kubectl --context docker-desktop -n ayni-orchestrator-dev rollout status deployment/provider-fixture
kubectl --context docker-desktop -n ayni-orchestrator-dev port-forward service/provider-fixture 18080:8080
```

Configure the local chart with `ayni-orchestrator.providers.openai.enabled=true`,
`ayni-orchestrator.providers.openai.fixtureEgress=true` and
`ayni-orchestrator.providers.openai.baseUrl=http://provider-fixture.ayni-orchestrator-dev.svc:8080/v1`.
Create the local authority bearer with `dev/scripts/configure-development-secrets.sh`
before upgrading. Start `dev/ctl console` and run:

```sh
PROVIDER_FIXTURE_URL=http://127.0.0.1:18080 \
  node apps/console/provider-acceptance.mjs
```

The acceptance script creates an isolated workspace and a workspace-scoped
Kubernetes Secret containing only the public dummy credential. It uses the
explicit `docker-desktop` context and `<namespace>-sessions`; set
`AYNI_ORCHESTRATOR_NAMESPACE` to select another local deployment namespace.
It preserves accepted application data and retained ledgers.

`fixture:no-usage` omits token usage, `fixture:disconnect` closes the connection
after accepting the request, and `fixture:delay` delays the result for a
cancellation race. Other inputs return a completed response with reported usage.
`GET /stats` exposes counts and correlation IDs for proving that replay does not
make another provider request. These fixtures establish protocol and recovery
behavior, not live-provider latency or availability.

## Bounded tools and recovery

`fixture:tools` requests the three permitted scoped reads in one response. The
continuation must contain their matching call IDs and bounded JSON outputs. The
fixture rejects provider-native tools, additional capabilities and mismatched
outputs. It keeps only counts and correlation IDs, including a
`toolContinuations` counter; it does not retain tool output.

The acceptance scenario also submits `fixture:tool-scope` (a model-supplied
conversation ID), `fixture:tool-limit` (nine invocations), `fixture:tool-args`
(over 4 KiB arguments) and `fixture:tool-unknown` (a shell capability). Each must
fail explicitly without a second provider request or assistant response. These
are malicious provider fixtures, not newly granted capabilities. The scenario
checks accumulated usage, tool replay after reconnect and the explicit
possible-charge acknowledgement, including a lost acknowledgement response.

Run fixture protocol tests without a deployment:

```sh
node --test apps/console/provider-fixture.test.mjs apps/console/state.test.mjs
```

## Bounded live smoke

The live smoke uses one existing, idle conversation in an explicitly configured
OpenAI workspace. Provision the scoped credential through the documented session
mount; the smoke never reads, prints, copies or configures a credential. Select
an explicit model, a total turn allowance of at most 32,768 tokens and an output
allowance of at most 2,048 tokens. Provider pricing is not inferred from tokens.
The invocation below authorizes one possibly paid turn:

```sh
AYNI_ALLOW_PAID_PROVIDER_SMOKE=1 \
AYNI_SMOKE_WORKSPACE_ID=<workspace-uuid> \
AYNI_SMOKE_PROJECT_ID=<project-uuid> \
AYNI_SMOKE_CONVERSATION_ID=<conversation-uuid> \
  node apps/console/provider-live-smoke.mjs
```

It asks for the three scoped reads, waits for retained completion and checks
reconnect without another lifecycle. It requires retained evidence for all three reads. A model that declines them
fails the smoke; that is not a reason to retry the turn automatically. It prints only scope, receipt/call identity
and reported usage. It submits no automatic retry. If admission is ambiguous,
retain the printed key and use the canonical receipt recovery; if the provider
outcome is uncertain, preserve the request and ledger for explicit recovery.
A failed or unrun smoke is not live-provider evidence.

The deployed scenario also deletes two exact, ownership-checked session Pods in
its own workspace. After accepted tool completion, a later turn adopts the same
session generation and PVC without resending the completed request. For
`fixture:tool-restart`, the fixture returns one scoped read and pauses the
continuation response for 30 seconds. The scenario waits until that continuation
was received, deletes its exact worker Pod, and requires an uncertain outcome
with the same retained PVC and tool evidence. Reconnect must not resend the
continuation. No database, broker, session volume or retained history is deleted.


## Agent plan proposals

`fixture:proposal` returns a terminal `propose_plan` call with bounded content,
assumptions, unresolved questions and declared evidence. `fixture:proposal-malformed`
uses an invalid task key, `fixture:proposal-scope` adds an unauthorized conversation
selector, and `fixture:proposal-mixed` combines a proposal with a read call. Each
invalid response must fail visibly without creating a plan or another paid call.
`fixture:proposal-delay` provides the cancellation boundary.

The existing provider acceptance script now checks immutable version provenance,
review after reload, escaped content, scope rejection and cancelled proposals.
The same server accepts the scoped-read request profile with the additional
strict proposal capability. It never contacts a real provider.

For a separate bounded live proposal smoke, run the existing live script with
`AYNI_SMOKE_PLAN_PROPOSAL=1` in addition to its documented scope and paid-smoke
opt-in variables. It requests one tiny terminal proposal, verifies its originating
turn/provider result and confirms the same version after reload. Use a fresh idle
conversation in the explicitly enabled live workspace; credentials stay mounted
in that workspace's isolated session. See the
[proposal contract](../../../docs/technical/architecture/agent-proposals.md).
