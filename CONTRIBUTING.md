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
