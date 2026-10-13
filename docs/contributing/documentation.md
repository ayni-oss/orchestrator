# Documentation authoring

Documentation is Markdown under `docs/`, rendered with VitePress using the CLI
site's theme, Mermaid diagrams and local search. It uses the pinned Node and pnpm
versions and the root `pnpm-lock.yaml`.

```sh
pnpm install --frozen-lockfile
pnpm docs:dev
pnpm docs:check
pnpm docs:preview
```

## Where content belongs

| Directory | Content |
| --- | --- |
| `guides/` | Product flows and operator tasks; label unimplemented direction explicitly |
| `architecture/` | Current execution, trust and code boundaries |
| `operations/` | Deployment, recovery, upgrades and compatibility procedures |
| `reference/` | Supported integration, access, event and telemetry contracts |
| `contributing/` | Contributor workflows |
| `decisions/` | Dated decision records and supersession links |
| `history/` | Dated research and source snapshots |

Use relative links that work when reading the repository. The site rewrites links
to files outside `docs/` into GitHub source links and publishes the archived CSV
as a download. Update sidebar entries in `.vitepress/config.mts` when adding a
current guide. Keep generated protocol bindings owned by their existing tooling.

`pnpm docs:check` checks local Markdown targets and builds the complete site with
dead-link detection. It does not verify external service availability. PR validation
runs this check with a frozen dependency install.

## Publication

The documentation workflow deploys default-branch documentation after merge and
supports manual recovery through `workflow_dispatch`. PR builds do not deploy.
Configure GitHub Pages to use GitHub Actions and set the custom domain to
`orchestrator.ayni.build`; the maintainer manages DNS. The committed `public/CNAME`
and sitemap use that hostname. Verify the domain before enforcing HTTPS.

Requirements, milestones and implementation plans belong in GitHub issues and the
organization Project. Completed plans stay in Git history or a dated research
archive; current guides describe supported behavior.
