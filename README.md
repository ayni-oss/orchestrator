# Ayni Orchestrator

Ayni Orchestrator is a private development-workflow control plane that produces
reviewable repository changes on controlled infrastructure. Its pre-public v1
is being replaced by one shared foundation with **Ayni Managed** and **Custom
Runtime** runtime profiles. Ayni Managed is the first complete profile; Custom
Runtime is a named later follow-up. The normative design is
[Decision 0007](docs/decisions/0007-clean-v1-runtime-profiles.md).

The checked-in pilot API, SQLite state, local preparation archives, and
validation lifecycle are retired design history. They are replaced directly,
without a migration, compatibility API, or legacy-read path by delivery issues
[#53–#68](https://github.com/ayni-oss/orchestrator/issues/51). This repository
remains private; Ayni is independently useful as a local CLI in the public
[`ayni-oss/ayni`](https://github.com/ayni-oss/ayni) repository.

## Target product

The shared core owns repository connections, verified OCI runtime artifacts,
agent attempts, candidates, PR/MR publication, assessments, evidence identity,
and reconciliation. Ayni Managed adds committed Ayni lock interpretation,
GitHub Actions automation, GHCR environment publication, advisory host checks,
managed exact-head PR assessment, signals, and default-branch environment
reconciliation. Custom Runtime later supplies a verified external OCI image
without Ayni-specific fields or negative branding.

A task freezes a repository revision, profile revision, and verified OCI digest.
A candidate commit, pull-request head SHA, workflow run, and evidence digest are
different identities. Plain `ayni check` in the launched verified image is agent
feedback; managed PR execution is the quality gate; GitHub checks/comments are
presentation; digest-verified evidence and OCI digests are retained identities.

## Documentation

| Document | Purpose |
| --- | --- |
| [Clean-v1 runtime-profile decision](docs/decisions/0007-clean-v1-runtime-profiles.md) | Normative names, ownership, identity, lifecycle, trust, recovery, and issue map |
| [Architecture](docs/architecture.md) | Target shared/profile component and resource model |
| [Protocol contract](proto/README.md) | Target clean-v1 gRPC resources and compatibility reset |
| [Local development](docs/local-development.md) | Target Docker Desktop workflow and validation boundary |
| [Operations](docs/operations.md) | Target readiness, recovery, retention, and evidence operations |
| [Roadmap](ROADMAP.md) | Linked delivery issues and follow-ups |
| [Local Helm deployment decision](docs/decisions/0006-local-helm-chart.md) | Existing local chart/deployment boundary; worker details are historical until revised |
| [Ayni 0.14 integration contract](docs/ayni-0.14-integration.md) | Released launcher, certification, and assessment-evidence boundaries |
| [Custom-client gRPC access](docs/custom-client-access.md) | Private endpoint boundary; not public hosting authorization |

## Planning and delivery

Requirements and delivery status live in GitHub rather than duplicate planning
documents:

- [Ayni Orchestrator project](https://github.com/orgs/ayni-oss/projects/2)
- [Orchestrator issues](https://github.com/ayni-oss/orchestrator/issues)
- [Completed milestones](https://github.com/ayni-oss/orchestrator/milestones?state=closed)

Repository development rules and the private boundary with Ayni are recorded in
[AGENTS.md](AGENTS.md). Public Orchestrator announcements, documentation,
releases, packages, images, or links from Ayni require an explicit publication
decision.
