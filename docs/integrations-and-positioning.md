# Agents, integrations, and positioning

Status: researched options and product hypotheses, 2026-09-15. External features
can change. These references are a dated baseline, not a feature-parity guarantee.

## Competitive baseline

Cursor's Projects announcement overlaps substantially with the envisioned user
experience. Compare the whole documented product surface rather than assuming
that agent providers only offer a model and a repository.

The Ayni column below combines existing local contracts with explicitly proposed
orchestration. It does not imply an implemented competing platform.

| Dimension | Cursor documentation | Ayni plus proposed orchestrator |
| --- | --- | --- |
| Persistent work | Coordinator, shared context, delegation, and recurring work. [Projects](https://cursor.com/blog/projects) | Proposed durable projects and tasks tied to repository contracts |
| Event-driven tasks | Slack, schedules, and PR subscriptions. [Projects](https://cursor.com/blog/projects) | Proposed provider-neutral routing and notifications |
| Prepared environments | Dockerfiles, installation scripts, and reusable builds. [Setup](https://cursor.com/docs/cloud-agent/setup) | Existing Ayni locked inputs; proposed build and readiness service |
| Multiple repositories | Inspect, change, and test several repositories in one environment. [Setup](https://cursor.com/docs/cloud-agent/setup) | Proposed composition of independently prepared repo environments |
| Environment evolution | Builds record repo commits; branch runs reuse prepared state and can refresh dependencies. [Builds](https://cursor.com/docs/cloud-agent/builds) | Proposed candidate preparation and validation before baseline promotion |
| Customer infrastructure | Self-hosted workers and team pools, including Kubernetes; agent loop remains in Cursor cloud. [Runtime choices](https://cursor.com/docs/cloud-agent/self-hosted/choose-runtime) | Intended self-hosted orchestration and execution, with configurable model provider |
| Acceptance evidence | Reviewed pages describe testing and CI workflows but do not establish an equivalent typed quality contract | Existing Ayni findings, exact verification commands, provenance fields, and completion accounting |
| Agent implementation | Cursor's agent experience | Intended replaceable agent integration; a Rig-based runtime is an option |

The evidence comparison describes what the reviewed pages establish, not proof
that Cursor lacks other quality features. Do not claim that webhooks, multi-repo
support, prebuilt environments, or Kubernetes execution are unique.

The product hypothesis is the combination of open-source operational ownership,
agent choice, portable quality/environment contracts, and evidence for coordinated
changes. Cursor can also invoke Ayni, which makes integration a first-class path.

Other relevant categories include configured agent environments such as
[Devin](https://docs.devin.ai/onboard-devin/environment), self-hosted agent
workspaces such as [Coder](https://coder.com/docs/ai-coder/agents), and programmable
sandbox infrastructure such as [Daytona](https://www.daytona.io/docs/snapshots/).
Treat these as potential integrations as well as alternatives; no exhaustive
market comparison or validated differentiation claim has been made.

## Cursor integration

### Level 1: Ayni in a Cursor workspace

Use the repository's Ayni guidance and CLI as the initial integration surface:

1. Prepare a compatible runtime and inspect the repository contract.
2. Have Cursor run focused verification while implementing changes.
3. Return normalized findings and their exact verification commands.
4. Run the full repository contract at the completion boundary.

Cursor documents `AGENTS.md` instructions and configurable development
environments. That provides an integration surface, not verified support for
Ayni's managed backend. Test the selected environment's engine availability,
mounts, snapshot behavior, executor compatibility, and evidence output.
[Cursor setup](https://cursor.com/docs/cloud-agent/setup)

No MCP layer is required for the initial CLI experiment. A thin MCP interface
could later expose contract inspection, verification, and typed findings. It
should use Ayni's existing semantics rather than recreate policy evaluation.

### Level 2: Cursor with orchestrator-managed workers

Cursor's self-hosted execution allows tool calls to run on customer machines while
the agent loop remains in Cursor's cloud. This is a concrete direction to test for
orchestrator-provisioned workspaces.
[Runtime choices](https://cursor.com/docs/cloud-agent/self-hosted/choose-runtime)

Unverified questions include worker registration and lifecycle APIs, licensing,
capability availability for Projects, startup/reset behavior, workspace layout,
and whether cross-repo commands can target independent environments. Do not
assume Cursor can consume the proposed workspace gateway unchanged.

A combined worker with multiple repo checkouts may be the easiest initial path.
Separate repo pods may need a tool adapter or a different agent integration.
Validate actual file edits, terminal execution, Ayni managed checks, and
conversation continuity before advertising support.

## Rig as an optional native runtime

Rig is a Rust agent-building library with provider abstractions, agent execution,
typed tools, and MCP integration through `rmcp`.
[Rig API](https://docs.rs/rig/latest/rig/),
[MCP integration](https://docs.rs/rig/latest/rig/agent/tool/rmcp/index.html)

Its proposed role is model interaction, tool calling, consuming findings, and
producing edits. The application still needs durable task state, budgeting,
checkpoint policy, context management, access controls, and recovery of external
actions. Do not infer a complete reliable coding product from library primitives.

Keep the agent integration boundary independent of Rig. First compare a thin
native runtime with an existing coding-agent integration on real tasks. Pin a
tested version if adopted; linked `latest` documentation is a research reference,
not a dependency constraint.

## Agent profiles, skills, tools, and MCP

An illustrative resolved profile contains:

- Agent implementation and model configuration.
- Versioned instructions and skills, with source references.
- Available tools and MCP servers, plus connection configuration.
- Credential references and capabilities authorized for the task.
- Time, concurrency, and spending budgets.

Separate organization policy, project defaults, repository guidance, and task
overrides. Exact precedence is open, but a less-trusted layer must not grant
itself permissions withheld by the orchestrator. Skills guide behavior; they do not
grant credentials or override required acceptance policy.

Record the resolved profile for each attempt. Repository-owned stdio MCP tools
may need to run in the repo environment; remote MCP services have separate
authentication and network requirements. Do not bake organization secrets or
every agent integration into every Ayni environment image.

## Provider and notification interfaces

Start from one provider rather than promising simultaneous feature parity across
GitHub, GitLab, and others. Preserve provider-neutral internal identities while
allowing provider-specific installation and authorization semantics.

Incoming adapters should validate authenticity, normalize event identity, map
the authenticated caller to project access, deduplicate deliveries, and decide
whether an event refreshes readiness, starts a task, or continues one. Revocation
must affect future operations and active credentials according to policy.

Notifications should reference the same durable task across Slack, email, UI,
and provider comments. Keep destination authorization and delivery state outside
the agent's free-form response. Emit useful state changes rather than a message
for every reconciliation loop, and avoid feedback loops from the bot's own events.

## UI hypothesis

An initial UI can focus on projects, environment readiness, tasks, conversation,
proposed changes, and evidence. Alternative entry points should reuse that task
model. When Cursor owns the conversation, the orchestrator can still expose readiness,
resource state, and evidence without requiring users to adopt a second chat UI.
