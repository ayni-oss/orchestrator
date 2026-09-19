# Ayni Orchestrator — Agent Rules

## Private development boundary

- This repository is private development and exploration. Keep its visibility
  private until the user explicitly decides to publish it.
- Keep orchestrator plans, architecture, terminology, integrations, examples, and
  roadmap in this repository. Do not expose them through public Ayni code,
  documentation, issues, PRs, commit messages, releases, or cross-repository links.
- Ayni must remain independently useful as a local environment and quality tool.
  Orchestrator-driven discoveries may motivate Ayni fixes and improvements only when
  they have an independently justified Ayni use case and respect its boundaries.
- Describe any such upstream improvement truthfully in terms of Ayni behavior,
  with standalone tests and documentation. Do not conceal orchestrator-specific
  functionality behind generic wording; keep that functionality here.
- Keep Kubernetes controllers, provider integrations, project/task lifecycle,
  agent orchestration, and orchestrator access policy here. Do not add a dependency
  from Ayni to this private repository or require private fixtures for Ayni CI.
- Changes to Ayni belong in a separate branch and PR in its repository, following
  its own AGENTS.md. Do not silently edit Ayni as part of an orchestrator milestone.
- Verify remote visibility before pushing private material. Do not create public
  packages, images, releases, sites, announcements, or publishing workflows
  without a separate explicit publication decision.

## Implementation direction

- Prefer Rust for orchestrator-owned application code, including controllers and
  workers. Kubernetes controllers and Operators do not require Go. Evaluate kube-rs for the
  controller runtime; pin tested dependencies when implementation begins.
- Rust does not require reimplementing infrastructure, language tools, or external
  agents. Keep those integrations behind explicit boundaries.
- Read README.md and the relevant documents in docs/ before implementation.
  Distinguish user decisions, proposals, experiments, and verified capabilities.
- Use docs/mvp.md for initial product scope and docs/mvp-validation.md for
  engineering gates and pilot goals. Broader exploration documents do not expand
  the MVP implicitly, and an execution spike alone does not complete the MVP.
- Follow docs/milestones.md: one delivery PR per milestone, starting from updated
  main after the preceding milestone merges. Record completion evidence and update
  the milestone status; do not merge automatically.
- Deploy working behavior to Docker Desktop Kubernetes from M0 onward, during
  each deployable milestone. Use Kustomize and the explicit docker-desktop context
  as defined in docs/decisions/0001-local-kubernetes-deployment.md. Never fall back
  to another current cluster when the local context is absent. Local deployment
  is authorized; destructive cluster resets and retained-data deletion are separate.
- Keep the first iteration small and end-to-end. Define its observable outcome
  and failure/recovery cases before expanding resources or integrations.
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
- Start new milestones from updated main on a fresh branch. Do not reuse a merged
  branch or implement directly on main.
- After repository changes, commit and push unless explicitly instructed otherwise,
  and open a PR with an accurate scope and validation results.
- Once executable code exists, pin the Rust toolchain and run formatting, Clippy,
  tests, and compile checks with that toolchain, plus required CI checks locally.
  Exercise affected Kubernetes behavior on a disposable cluster before claiming
  it is validated. Report blocked checks; do not treat mocks as cluster evidence.
- Documentation-only changes require review, whitespace checks, and validation of
  local document links. Do not invent passing Rust or Kubernetes checks while the
  repository contains no implementation.

## Rust guidance

- Apply the pinned `rust-skills` guidance at `.agents/skills/rust/SKILL.md` when
  writing or reviewing Rust. Its complete upstream rules are vendored alongside
  the skill; attribution and the preserved MIT license are in
  `third_party/rust-skills/`.
