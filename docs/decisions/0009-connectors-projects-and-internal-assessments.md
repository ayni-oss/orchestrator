# Decision 0009: connectors, projects, and internal assessments

- Status: accepted; membership, initialization, and registry configuration amended
  by [Decision 0011](0011-project-centered-ui-and-onboarding.md)
- Date: 2026-10-04
- Supersedes: the repository-first product journey and Actions-authoritative
  assessment model in Decision 0007, and the default producer selection in
  Decision 0008. Existing isolation and evidence-integrity requirements remain.

## Product boundary

The product journey is **Connectors → Projects → Tasks**. A connector represents
a customer-owned VCS App and its installation access. A project is a separately
identified, named workspace. Its repository connection and runtime profiles are
implementation resources, not substitutes for the project identity.

Membership is stored separately from the project. This release enforces exactly
one repository per project and at most one active project per immutable provider
repository. Membership cannot be edited. Archive releases the active membership
claim, preserves history, and prevents new task admission and retries. Existing
work is not cancelled. Re-onboarding creates a new project with distinct history.

Tasks belong to projects. Admission resolves the member repository and verifies
any supplied repository assertion and selected profile before freezing source,
profile revision and runtime digest. Listing by project must not include tasks
from another project that previously used the same repository.

A repository must already contain valid committed `.ayni.toml` and `.ayni.lock`.
Orchestrator does not initialize, rewrite, repair, or weaken Ayni configuration,
instrumentation, lock inputs, or certificate trust. It consumes supported Ayni
CLI and artifact contracts. Missing inputs require repository-owner action.

## Credentials and bootstrap

The application starts without a GitHub App. Create-App and JSON-import flows
validate and persist credentials dynamically. Operational connector credentials
are encrypted using authenticated encryption; the deployment-managed encryption
root remains outside the database. Ordinary APIs expose safe metadata only.
The creation callback may provide a one-time download of the original bundle;
existing stored private keys are not browser-readable. Replacement and disabling
invalidate cached provider authorization without redeploying the application.

Helm references existing namespace Secrets for registry pull, registry push,
environment signing, build-engine TLS, model access, and connector encryption.
Registry and signing material is neither stored in the database nor read by the
control-plane process. Pull and push may reference the same Secret. Build-engine
provisioning remains an operator responsibility.

## Project readiness and environment production

Project inspection records the exact observed default-branch SHA, installation
access, Ayni inputs, supported platform, instrumentation, producer trust, owned
integration files, and environment verification. Lifecycle (active/archived) is
separate from readiness. Inaccessible images and invalid signatures are not
missing-image observations and do not authorize builds.

Create one deterministic setup PR only if owned integration files need changes;
skip it when none do. GitHub workflows are not required. Preserve unrelated files
and human edits; never merge automatically. Reinspect the merged SHA.

Reuse a compatible certified environment or build a genuinely missing one in a
dedicated internal Job using the supported Ayni CLI. Publish to the private
`ghcr.io/<owner>/<repository>-ayni` package with repository source metadata.
Build identity binds immutable inputs, lock, platform and producer configuration.
Discovery tags locate images; independent verification admits immutable digests.
Source-only changes reverify without rebuilding. In-flight tasks keep their
frozen digest. Persist build intent and reconcile uncertain publication outcomes
before deciding whether another build is allowed.

Only trusted producer workloads receive signing and push credentials. Repository
policy must already pin the producer public key. Candidate-controlled policy or
lock changes never authorize automatic certification.

## Coding and authoritative verification

The agent runs Ayni during development inside the certified environment. Save a
checkpoint commit before final verification. A separate verifier checks a fresh
checkout of that commit in the same image digest, with original signals retained
outside candidate-writable storage. Feed actionable failures back to the agent,
commit repairs, and verify the new commit. Allow at most three repair cycles
within the original task deadline and model budget. Do not create empty commits.

Bind evidence to project, task, attempt, candidate/base commit, verifier Job,
profile revision, runtime digest, lock, configuration, platform, Ayni version and
artifact digests. The verifier receives no model, signing, registry-push or App
private-key credentials. Checkout authentication is confined to initialization.
Ayni owns policy and certificate interpretation.

Publish valid candidates as drafts, including failed or incomplete work with
truthful status and bounded Ayni Markdown in an owned PR-description section.
No-change work produces no PR; cancellation prevents new automatic publication.
Keep structured evidence internally. Changed PR heads stale earlier assessments
and trigger supported internal reassessment, not automatic agent repair. Default
publication remains draft-only; optional readiness requires successful complete
evidence for the current head. GitHub Actions are not an assessment authority.

## Fresh-install contract

Schema version 3 establishes projects and task ownership. Databases with previous
schema versions are rejected without migration or automatic data deletion. Use
an isolated namespace and new storage for deployment validation. Never reset an
existing deployment merely to satisfy this requirement.

The native ProjectService uses the existing `repositories.read` and
`repositories.write` capabilities. Create, rename and archive are idempotent;
rename/archive require the observed resource version. Project list pagination
retains the shared stable-snapshot contract. Native task creation requires
`project_id`; its optional repository assertion cannot select another member.

Requirements and delivery are tracked in the private
[product epic](https://github.com/ayni-oss/orchestrator/issues/90) and its linked
milestones. This decision defines the accepted architecture, not a claim that
all dependent product flows are operational.
