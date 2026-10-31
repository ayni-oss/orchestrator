# Ayni Orchestrator web application

This is the only web application. Its shell, design tokens, typography, theme toggle,
provider pages and shared components were promoted from the former example at
[the original source revision](https://github.com/ayni-oss/orchestrator/tree/c279a7bcdf340f5479cc292c1571bba22af04b27/apps/web).
The duplicate example and standalone diagnostic frontend have been removed.

Canonical APIs support workspace/project creation, manual GitHub App credentials,
GHCR PAT connections, OpenAI key validation/model discovery and explicit workspace
connection/model/budget selection. Historical BFF/API wiring is not restored.
Shared GitHub repository discovery/catalog and direct project associations are available.
Exact-commit static Ayni inspection provides retained findings and retry recovery.
Guided App setup, full adapter discovery and runtime preparation remain outstanding.

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

Repository catalog UI: `/repositories`. Project cards link to
`/projects/:id/repositories?workspace=:workspaceId`. Repository connect and project
association requests retain their exact non-secret bytes and idempotency key across
reload; scope changes create a fresh component instance. **Connected** describes
historical source access and never authorizes task execution.

### Workspace and project navigation

The sidebar provides a workspace dropdown with a separate create button, and an
always-visible workspace settings link that is disabled without a selection.
The home page lists workspace cards; opening one lists its project cards. Both
use a whole-card link and a separate top-right three-dot menu for repositories
and rename. Workspace menus also open workspace settings. The sidebar groups
Repositories and Configuration under Settings at the bottom. Sidebar items share
one component for hover, focus, selected and disabled states. A single Projects
parent opens the project list, with indented links to each workspace project below
it and a More projects link when another page is available.

Renames retain an exact request in session storage until acknowledged. The server
checks the displayed resource version, records an immutable receipt, and rejects
stale edits. Retrying an acknowledged rename returns its original receipt without
reapplying the old name over a later edit.

### Inspect committed Ayni setup

In Repositories, open **Inspect Ayni setup** on a connected repository and enter a
full lowercase commit SHA. The retained result shows policy/lock digests, locked
tools, public trust key IDs and corrective findings. Reload after an interrupted
request and choose **Recover inspection** or **Retry same inspection**.

Static inspection never enables preparation or starts work. Full adapter discovery,
configured signer admission and runtime verification remain later gates.

### Runtime signer references

Configuration → Runtime signing manages public signer identities and references to
administrator-owned immutable Secrets. Exact updates and original receipt recovery
survive reloads. Saving a reference validates public input shapes; it does not read
the Secret, grant signing access, enable builds or establish runtime readiness.
The runtime preparation execution journey remains under development.
