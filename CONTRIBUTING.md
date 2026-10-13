# Contributing

Contributions are submitted through pull requests. Keep changes focused, add
appropriate tests, and ensure the required checks pass.

## Pull requests

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
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo check --workspace --all-features
pnpm install --frozen-lockfile
pnpm frontend:check
pnpm test:frontend
pnpm docs:check
```

Run affected protocol, Helm and repository quality checks described by
`.github/workflows/pr-validation.yml` and `.ayni.toml`. Runtime behavior changes
also require the explicit Docker Desktop Kubernetes topology.

## Documentation

Use [the documentation index](docs/index.md) to find current guides.
[Documentation authoring](docs/contributing/documentation.md) describes the
source layout, link checks and publication workflow. Requirements and delivery
status belong in issues and the organization Project.
