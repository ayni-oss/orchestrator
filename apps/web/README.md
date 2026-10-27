# Ayni Orchestrator web application

This is the only web application. Its shell, design tokens, typography, theme toggle,
provider pages and shared components were promoted from the former example at
[the original source revision](https://github.com/ayni-oss/orchestrator/tree/c279a7bcdf340f5479cc292c1571bba22af04b27/apps/web).
The duplicate example and standalone diagnostic frontend have been removed.

Canonical APIs support workspace/project creation, manual GitHub App credentials,
GHCR PAT connections, OpenAI key validation/model discovery and explicit workspace
connection/model/budget selection. Historical BFF/API wiring is not restored.
Guided App setup, the repository catalog and recoverable registry artifact probes
remain outstanding.

Run `pnpm web:dev` with the local gateway, or `dev/ctl live` for one frontend plus
backend rebuild/rollout. The default web URL is http://127.0.0.1:5174.
Configuration uses the original `/settings/source-control` and
`/settings/source-control/github` routes, alongside `/settings/container-registries`,
`/settings/llm-providers` and `/workspace/settings`. Light/dark themes use the example's
design tokens and locally bundled fonts, with the input token mapped to the border color so form controls stay visible in both themes.

## Shared UI components

Use shadcn-svelte components from `src/lib/components/ui` for interactive controls,
including Button, Input, Textarea, Dialog and NativeSelect. Keep theme tokens in
`src/app.css`; `--input` is a control border color, not a field background.
`components.json` configures the CLI aliases, Tailwind CSS entrypoint and icon library.

NativeSelect is installed from the official shadcn-svelte source at
[`6149e141`](https://github.com/huntabyte/shadcn-svelte/tree/6149e14107a9de3530a3083de2f1b7623b3c5d1f/docs/src/lib/registry/ui/native-select),
with the upstream Nova style utilities expanded into component classes and its icon
resolved to the existing Lucide dependency. Other controls use the same compact
shadcn styling. Update shared components and tokens instead of adding page-specific
field borders or raw form controls.
