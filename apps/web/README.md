# Ayni Orchestrator web application

This is the only web application. Its shell, design tokens, typography, theme toggle,
provider pages and shared components were promoted from the former example at
[the original source revision](https://github.com/ayni-oss/orchestrator/tree/c279a7bcdf340f5479cc292c1571bba22af04b27/apps/web).
The duplicate example and standalone diagnostic frontend have been removed.

Current canonical APIs support workspace/project creation and manual GitHub App
configuration. Historical BFF/API wiring and unsupported screens are not restored.
Repository catalog, registry configuration and OpenAI selection follow separately.

Run `pnpm web:dev` with the local gateway, or `dev/ctl live` for one frontend plus
backend rebuild/rollout. The default web URL is http://127.0.0.1:5174.
Configuration uses the original `/settings/source-control` and
`/settings/source-control/github` routes. Light/dark themes use the example's
unchanged design tokens and locally bundled fonts.
