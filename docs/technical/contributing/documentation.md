# Documentation authoring

Documentation serves two audiences. Keep user guidance and implementation detail
in their own source trees; only the user guide is published.

| Source | Audience | Content |
| --- | --- | --- |
| `docs/site/` | People using Ayni Orchestrator | Setup, repositories, tasks, results, trust and recovery |
| `docs/technical/` | Contributors, deployment maintainers and coding agents | Architecture, code ownership, contracts, deployment procedures and research |
| `docs/index.md` | Repository readers | Entry points for both audiences |

## Human guide

Explain what a person can do and what result to expect. Keep each page focused on
one journey, use diagrams for relationships and branching flows, and put settings
or results in compact tables. Do not include database schemas, event codecs,
controller internals or implementation proposals in user journeys.

The site uses the CLI VitePress theme, Mermaid and local search. Its only source
directory is `docs/site/`; technical documents are excluded from rendering,
navigation, search and the sitemap. Public assets live in `docs/site/public/`.
Navigation is defined in `docs/.vitepress/config.mts`.

Every Mermaid fence starts with this configuration, also enforced by the link check:

```yaml
---
config:
  theme: neutral
  look: classic
  layout: elk
---
```

The theme disables Mermaid’s automatic page-load scanner before loading ELK;
the VitePress components alone render the authored diagram source. This prevents
the scanner from parsing empty containers during initial loading. SVG text labels size
nodes naturally with padding; wide diagrams scroll rather than shrinking text.
Review labels inside both rectangles and decision diamonds after changing a flow.

## Technical sources

Document current contracts precisely and keep canonical code, schema and command
references. Use diagrams to explain authority, state changes and recovery; retain
necessary bounds and compatibility rules in tables or focused reference sections.
Label dated research and unimplemented direction explicitly. Requirements and
milestone status remain in GitHub issues and the Project.

## Validate and publish

```sh
pnpm install --frozen-lockfile
pnpm docs:dev
pnpm docs:check
pnpm docs:preview
```

`docs:check` validates repository Markdown targets, builds the human guide with
dead-link detection, and checks that the output contains only human-source pages.
It also checks the custom domain. PR validation runs a frozen dependency install;
PRs do not deploy. External service availability is not part of the link check.

The Pages workflow deploys main after merge and supports manual recovery.
GitHub Pages uses Actions with the `orchestrator.ayni.build` domain. The maintainer
manages DNS and enables HTTPS after domain verification. Technical sources remain
available through GitHub; they are not a second website.
