# Former web UI example

Reference source: [apps/web at c279a7bcdf340f5479cc292c1571bba22af04b27](https://github.com/ayni-oss/orchestrator/tree/c279a7bcdf340f5479cc292c1571bba22af04b27/apps/web).

This directory preserves the former SvelteKit application as a visual
and interaction reference. Its original README describes that historical app. The only source adjustment is
trailing-whitespace cleanup in `src/routes/tasks/[taskId]/+page.svelte`.
It is excluded from the active pnpm workspace, builds, CI, deployment and docs site.
It is not connected to the current backend and is not a supported runnable app.
Its workspace dependencies (including the former BFF), generated contracts and
backend APIs belong to that source revision and are intentionally not restored.

Use the layouts, components, styles and assets when designing the future product UI.
Do not treat its API clients, authority model or feature availability as current
contracts. Backend work and the remaining roadmap continue independently.
