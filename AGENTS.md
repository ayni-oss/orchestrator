# Ayni Orchestrator — Agent Rules

## Repository and publication boundary

- This repository is public. The maintainer has authorized publication of
  orchestrator architecture, implementation, documentation, issues and pull requests.
- Keep durable orchestrator architecture, operations, terminology, and examples
  in this repository. Use ROADMAP.md for agreed direction and delivery milestones;
  each pull request is its own work item with requirements and delivery evidence.
  Never publish credentials or private operational data.
- Ayni must remain independently useful as a local environment and quality tool.
  Orchestrator-driven discoveries may motivate Ayni fixes and improvements only when
  they have an independently justified Ayni use case and respect its boundaries.
- Describe any such upstream improvement truthfully in terms of Ayni behavior,
  with standalone tests and documentation. Do not conceal orchestrator-specific
  functionality behind generic wording; keep that functionality here.
- Keep Kubernetes controllers, provider integrations, project/task lifecycle,
  agent orchestration, and orchestrator access policy here. Do not add a dependency
  from Ayni to this repository or require private fixtures for Ayni CI.
- Changes to Ayni belong in a separate branch and PR in its repository,
  following its own AGENTS.md. Link related PRs across the repositories.
  Do not silently edit Ayni as part of an Orchestrator change.
- Repository publication does not authorize new package/image releases, sites,
  announcements or publishing workflows; those require a separate maintainer decision.

## Ayni domain

- Ayni's canonical public domain is `ayni.build`, not `ayni.io`.
- All Ayni event types use the reverse-DNS prefix `build.ayni`, derived from
  `ayni.build`. Preserve historical `io.ayni` stored envelopes through
  compatibility readers and canonicalize their type names at delivery.

## Documentation audiences

- `docs/site/` is the human-facing user guide published at orchestrator.ayni.build.
  Keep pages short, explain user actions and outcomes, and use diagrams for flows.
- `docs/technical/` holds architecture, protocols, deployment internals, contributor
  rules and research for maintainers and coding agents. It is excluded from the site.
- Keep detailed implementation content out of user flows. Link each audience's index
  from the repository documentation index and validate the publication boundary.

## Implementation direction

- The project-workflow replacement is clean-slate. Prior SQLite workflow schemas,
  retained data, APIs, tests, deployment topology, and compatibility behavior are
  research input only and must not constrain the replacement. Do not add migration,
  dual-write, compatibility, or shared-broker work for them. The first M1 runtime
  slice deletes the superseded workflow implementation and tests as it installs the
  replacement boundary; subsequent work builds only on the replacement contracts
  and cumulative acceptance harness.
- Prefer Rust for orchestrator-owned application code, including controllers and
  workers. Kubernetes controllers and Operators do not require Go. Evaluate kube-rs for the
  controller runtime; pin tested dependencies when implementation begins.
- Rust does not require reimplementing infrastructure, language tools, or external
  agents. Keep those integrations behind explicit boundaries.
- Read README.md, ROADMAP.md, and relevant durable documents in docs/technical/
  before implementation. Each PR records its outcome, acceptance criteria,
  prerequisites, failure/recovery cases, and validation evidence, with a link to
  the relevant roadmap section. Do not require a separate GitHub issue or closing
  keyword. Do not merge automatically.
- Exercise deployable behavior on Docker Desktop Kubernetes when the change
  affects runtime behavior. Use the `dev/charts/ayni-orchestrator-local` Helm chart and the
  explicit `docker-desktop` context as defined in
  docs/technical/operations/local-development.md. Never fall back
  to another current cluster when the local context is absent. Local deployment
  is authorized; destructive cluster resets and retained-data deletion are separate.
- Keep each PR small and end-to-end. Define its observable outcome and
  failure/recovery cases before expanding resources or integrations.
- Consume Ayni's supported CLI and artifact contracts first. Do not duplicate its
  policy evaluation, adapter behavior, or lock interpretation in the orchestrator.
- A passing host check in a pod is not evidence of Ayni managed compatibility.
  Validate lock, executor/image identity, dependency preparation, source snapshot,
  completion accounting, and artifact collection on the actual worker topology.
- Keep final validation inputs frozen and evidence outside agent-writable storage.
  Retries must not duplicate work or external actions; stale results must not
  replace newer desired state. Test restart, cancellation, and cleanup behavior.

## Alpha interface versioning

- Keep every Orchestrator-owned API, event, schema and retained format at v1
  until alpha merges. Consolidate the canonical contract in place; do not create
  parallel v2/v3 interfaces or milestone-specific compatibility implementations.
- Immutable configuration revision IDs, conversation revisions, assignment/session
  generations and SQL migration ordinals identify changing data or installation
  steps. They are not interface versions and must still advance to preserve
  frozen inputs, ordering, fencing and retained-state recovery.
- Keep source contracts, generated bindings, consumers, fixtures and docs in sync.
  Preserve exact dependency pins and historical research provenance. Unsupported
  future-version rejection fixtures do not establish supported interfaces.
- Use the [conversation execution contract](docs/technical/architecture/conversation-execution.md)
  for the consolidated M2 configuration, session, provider and tool boundaries.

## Repository workflow

- Use descriptive, unbranded branches; never use codex/ or cortex/ prefixes.
- Use Conventional Commits for commits and PR titles.
- Fetch origin/alpha and start each change from its latest commit on a fresh
  branch. Target PRs explicitly at alpha (for example, gh pr create --base alpha),
  regardless of the repository default branch. Do not reuse a merged branch or
  implement directly on alpha or main. Repeat this after every merged PR.
- ROADMAP.md owns planned direction and milestone scope; PRs own individual
  delivery status and acceptance evidence. Update the roadmap in the same PR
  when direction or milestone status changes. Do not create duplicate GitHub
  issues for planned work.
- GitHub milestones are optional groupings: reuse a matching milestone, or create
  one when useful for an agreed roadmap milestone, and assign related PRs to it.
  A milestone does not replace the roadmap or require separate issues.
- After repository changes, commit and push unless explicitly instructed otherwise,
  and open a PR with an accurate scope and validation results.
- Sign every commit with the Developer Certificate of Origin using `git commit -s`.
- Once executable code exists, pin the Rust toolchain and run formatting, Clippy,
  tests, and compile checks with that toolchain, plus required CI checks locally.
  Exercise affected Kubernetes behavior on a disposable cluster before claiming
  it is validated. Report blocked checks; do not treat mocks as cluster evidence.
- Documentation-only changes require review, whitespace checks, and validation
  of local document links. Keep workflow guidance consistent with ROADMAP.md
  and CONTRIBUTING.md.

## Rust guidance

- Apply available Rust guidance when writing or reviewing Rust. Do not vendor
  agent skills or their supporting rule libraries into this repository.

## Credentials and access

- Never read, store, print, export, inspect, modify, or work around secrets.
- Never invoke 1Password, keychain, SSH agents, credential helpers, or
  authentication tooling unless the user explicitly authorizes that exact action
  in the current request.
- A request to push authorizes only use of the existing GitHub CLI session. Do
  not expose tokens or alter credential configuration.
- If authentication is unavailable, stop and report that fact. Do not retry via
  another secret source.
- Work only in repositories and GitHub organizations explicitly placed in scope;
  do not inspect, list, modify, or infer access to another organization.

## Change discipline and reporting

- Tests describe current behavior and supported compatibility, not implementation
  history. Remove tests whose only purpose is to preserve evidence of deleted code.
  Keep tests when they protect a current invariant, regression guarantee, migration,
  persisted-data contract, or API compatibility promise, and name them after that
  current requirement rather than the removed implementation or delivery phase.
  Keep completed-work provenance in Git history and linked issues or pull
  requests. When a compatibility window ends, remove its compatibility code and
  tests together.
- Preserve implementation unless the requested task requires code changes.
- Treat security boundaries, repository identity, provider behavior,
  control-plane safety, and account capabilities as first-class correctness
  concerns.
- Regenerate generated files with the project’s own tooling; never hand-edit
  fingerprints, locks, checksums, or generated documentation.
- Do not mechanically rewrite historical references when that makes provenance
  or old release links false.
- Distinguish a product failure from an upstream transient failure. Keep PR and
  change descriptions factual: scope, security impact, validation, and known
  follow-up work.
- Commit with a DCO sign-off and, where configured, a cryptographic signature.
