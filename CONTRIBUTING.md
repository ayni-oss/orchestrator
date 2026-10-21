# Contributing

Contributions are submitted through pull requests. Keep changes focused, add
appropriate tests, and ensure the required checks pass.

## Pull requests

Start each change on a fresh descriptive branch from the latest `origin/alpha`
and target the PR at `alpha` explicitly, even while the default branch is `main`.
Do not work directly on either integration branch or reuse a merged branch.

[ROADMAP.md](ROADMAP.md) owns planned direction and milestone scope. Each PR is
its own work item: link the relevant roadmap section and describe prerequisites,
observable outcomes, acceptance criteria, failure/recovery cases, and validation
results. A separate GitHub issue or issue-closing keyword is not required; do not
create duplicate issues for roadmap work. Update the roadmap when direction or
milestone status changes. Optional GitHub milestones can group related PRs;
reuse a matching milestone or create one for an agreed roadmap milestone when useful.

Use a Conventional Commit pull-request title:
`<type>(<optional-scope>)<optional-!>: <description>`. The title becomes the
squash-merge commit. CI rejects titles outside the supported `build`, `chore`,
`ci`, `docs`, `feat`, `fix`, `perf`, `refactor`, `revert`, `style`, and `test`
types.

Trusted metadata automation classifies the pull request for GitHub Projects:

- `feat` and `perf` use `kind/feature`;
- `fix` uses `kind/bug`;
- `docs` or a `docs` scope uses `kind/documentation`;
- a `security` scope uses `kind/security`; and
- all other supported types use `kind/chore`.

Classification does not assign priority. The automation owns those five
`kind/*` labels on pull requests and reapplies the classification when a title
changes.

## Alpha contracts

All Orchestrator-owned interfaces remain v1 until alpha merges. Extend the
canonical API, event, schema and retained format contracts in place rather than
adding parallel versions. Regenerate bindings and update consumers in the same
PR. Immutable configuration revision IDs, ordering revisions, executor/session
generations and SQL migration ordinals still advance: they identify data and
installation steps, not another interface. Exact dependency versions remain
pinned. See the [conversation execution contract](docs/technical/architecture/conversation-execution.md)
for the consolidated M2 decisions and retained-state rules.

## Developer Certificate of Origin

Every commit, including maintainer commits, must carry a valid DCO sign-off:

```bash
git commit -s
```

Configure cryptographic commit signing and use `git commit -S -s` when making
signed contributions. The DCO sign-off and a cryptographic signature serve
different purposes; both are required for protected branches.

## Local validation

Use the pinned toolchain in `rust-toolchain.toml`:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo check --workspace --all-targets
pnpm install --frozen-lockfile
pnpm typecheck
pnpm test
pnpm proto:check
pnpm chart:check
pnpm docs:check
scripts/ci/check-workflow-postgres-feasibility.sh
scripts/ci/check-workflow-private-v1-feasibility.sh
```

Run affected protocol, Helm and repository quality checks described by
`.github/workflows/pr-validation.yml` and `.ayni.toml`. Runtime behavior changes
also require the explicit Docker Desktop Kubernetes topology.

## Shared CI

CI consumes the public composite actions from `ayni-oss/ayni/.github/actions/`,
pinned to an immutable source revision with a `shared CI` comment. They provide
locked Node installs, public project file checks, title classification, managed
Ayni execution and PR quality feedback. Metadata and report jobs do not check out
PR code; only those jobs receive write permissions. Protocol, Helm, security and
integration checks remain specific to this repository.

## Documentation

Use [the documentation index](docs/index.md) to find current guides.
[Documentation authoring](docs/technical/contributing/documentation.md) describes the
source layout, link checks and publication workflow. Planned direction belongs in
the roadmap; individual delivery status and evidence belong in pull requests.
