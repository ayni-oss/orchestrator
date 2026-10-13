# Ayni Orchestrator — Agent Rules

## Repository and publication boundary

- This repository is public. The maintainer has authorized publication of
  orchestrator architecture, implementation, documentation, issues and pull requests.
- Keep durable orchestrator architecture, operations, terminology, and examples
  in this repository. Track requirements and delivery in the Orchestrator GitHub
  Project and linked issues. Never publish credentials or private operational data.
- Ayni must remain independently useful as a local environment and quality tool.
  Orchestrator-driven discoveries may motivate Ayni fixes and improvements only when
  they have an independently justified Ayni use case and respect its boundaries.
- Describe any such upstream improvement truthfully in terms of Ayni behavior,
  with standalone tests and documentation. Do not conceal orchestrator-specific
  functionality behind generic wording; keep that functionality here.
- Keep Kubernetes controllers, provider integrations, project/task lifecycle,
  agent orchestration, and orchestrator access policy here. Do not add a dependency
  from Ayni to this repository or require private fixtures for Ayni CI.
- Changes to Ayni belong in a separate issue, branch, and PR in its repository,
  following its own AGENTS.md. An Orchestrator issue may link the separate
  Ayni issue. Do not silently edit Ayni as part of an Orchestrator change.
- Repository publication does not authorize new package/image releases, sites,
  announcements or publishing workflows; those require a separate maintainer decision.

## Ayni domain

- Ayni's canonical public domain is `ayni.build`, not `ayni.io`.
- All Ayni event types use the literal `ayni.build` prefix. New DNS-qualified
  names also use `ayni.build`. Preserve historical stored envelopes through
  compatibility readers and canonicalize their type names at delivery.

## Implementation direction

- Prefer Rust for orchestrator-owned application code, including controllers and
  workers. Kubernetes controllers and Operators do not require Go. Evaluate kube-rs for the
  controller runtime; pin tested dependencies when implementation begins.
- Rust does not require reimplementing infrastructure, language tools, or external
  agents. Keep those integrations behind explicit boundaries.
- Read README.md, the linked issue, and relevant durable documents in docs/
  before implementation. The issue owns the outcome and acceptance criteria;
  repository documents describe current behavior and accepted decisions.
- Start each change from updated main on a focused branch and link its pull
  request to the issue with a closing keyword. Do not merge automatically.
- Exercise deployable behavior on Docker Desktop Kubernetes when the change
  affects runtime behavior. Use the `dev/charts/ayni-orchestrator-local` Helm chart and the
  explicit `docker-desktop` context as defined in
  docs/decisions/0006-local-helm-chart.md. Never fall back
  to another current cluster when the local context is absent. Local deployment
  is authorized; destructive cluster resets and retained-data deletion are separate.
- Keep each issue small and end-to-end. Define its observable outcome and
  failure/recovery cases before expanding resources or integrations.
- Consume Ayni's supported CLI and artifact contracts first. Do not duplicate its
  policy evaluation, adapter behavior, or lock interpretation in the orchestrator.
- A passing host check in a pod is not evidence of Ayni managed compatibility.
  Validate lock, executor/image identity, dependency preparation, source snapshot,
  completion accounting, and artifact collection on the actual worker topology.
- Keep final validation inputs frozen and evidence outside agent-writable storage.
  Retries must not duplicate work or external actions; stale results must not
  replace newer desired state. Test restart, cancellation, and cleanup behavior.

## Repository workflow

- Use descriptive, unbranded branches; never use codex/ or cortex/ prefixes.
- Use Conventional Commits for commits and PR titles.
- Start each issue from updated main on a fresh branch. Do not reuse a merged
  branch or implement directly on main.
- After repository changes, commit and push unless explicitly instructed otherwise,
  and open a PR with an accurate scope and validation results.
- Sign every commit with the Developer Certificate of Origin using `git commit -s`.
- Once executable code exists, pin the Rust toolchain and run formatting, Clippy,
  tests, and compile checks with that toolchain, plus required CI checks locally.
  Exercise affected Kubernetes behavior on a disposable cluster before claiming
  it is validated. Report blocked checks; do not treat mocks as cluster evidence.
- Documentation-only changes require review, whitespace checks, and validation
  of local document links. Requirements and delivery status belong in linked
  issues and the organization Project, not duplicated planning documents.

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
