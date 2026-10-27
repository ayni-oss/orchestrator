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
unchanged design tokens and locally bundled fonts.
