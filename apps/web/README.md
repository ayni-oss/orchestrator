# Ayni Orchestrator web application

This is the only web application. Its shell, design tokens, typography, theme toggle,
provider pages and shared components were promoted from the former example at
[the original source revision](https://github.com/ayni-oss/orchestrator/tree/c279a7bcdf340f5479cc292c1571bba22af04b27/apps/web).
The duplicate example and standalone diagnostic frontend have been removed.

Canonical APIs support workspace/project creation, manual GitHub App credentials,
GHCR PAT connections, OpenAI key validation/model discovery and explicit project
connection/model/budget selection. Historical BFF/API wiring is not restored.
Connect repositories in the shared GitHub catalog, then add them in Workspace settings.
Projects inherit the repositories available to their workspace.
Exact-commit static Ayni inspection provides retained findings and retry recovery.
Runtime preparation, signer setup, stage evidence and recovery are available behind
an explicitly qualified Kubernetes profile. Guided App setup, full adapter discovery
and project/task runtime handoff remain outstanding.
See the [alpha UI contract](../../docs/technical/architecture/project-workspace.md)
for project chat, plans, task graphs and delivery journeys still to implement.

Run `pnpm web:dev` with the local gateway, or `dev/ctl live` for one frontend plus
backend rebuild/rollout. The default web URL is http://127.0.0.1:5174.
Configuration uses the original `/settings/source-control` and
`/settings/source-control/github` routes, alongside `/settings/container-registries`,
`/settings/llm-providers` and `/workspaces/:workspaceId/settings`. Light/dark themes use the example's
design tokens and locally bundled fonts, with the input token mapped to the border color so form controls stay visible in both themes.

## Shared UI components

Follow the [product design manual](../../docs/technical/contributing/design-manual.md)
for titles, badges, icon actions, loading states and history presentation.

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
History controls use the [shadcn-svelte Pagination](https://www.shadcn-svelte.com/docs/components/pagination)
registry components for previous and next navigation.

Repository catalog UI: `/repositories`. Available repositories use the same compact, clickable grid cards as projects.
Opening a connected repository shows **Status** and **ayni** tabs. The header
shows connection status and reveals source integration details on hover or focus of its info icon.
Switching tabs preserves in-progress selections. Technical inspection
metadata is collapsed by default. Selecting a source control connection
automatically discovers repositories through its single active GitHub App
installation. Project cards link to
`/workspaces/:workspaceId/projects/:projectId/settings`. Repository connect and workspace
membership requests retain their exact non-secret bytes and idempotency key across
reload; scope changes create a fresh component instance. **Connected** describes
historical source access and never authorizes task execution.

### Workspace and project navigation

The sidebar provides a workspace dropdown with a separate create button. Open
Workspace settings using the three dots beside a workspace in the dropdown.
The dropdown and workspace cards offer the same actions.
The home page lists workspace cards; opening one lists its project cards. Both
use a whole-card link and a separate top-right three-dot menu. Workspace menus
open Workspace settings, where you can rename the workspace. Project menus
open Project settings for names, instructions, model selection and budgets. The sidebar groups
Repositories and Configuration under Settings at the bottom. Sidebar items share
one component for hover, focus, selected and disabled states. A single Projects
parent opens the project list, with indented links to each workspace project below
it and a More projects link when another page is available.

Renames retain an exact request in session storage until acknowledged. The server
checks the displayed resource version, records an immutable receipt, and rejects
stale edits. Retrying an acknowledged rename returns its original receipt without
reapplying the old name over a later edit.

### Inspect committed Ayni setup

In Repositories, open a connected repository card and choose **ayni**.
**Recheck default branch** resolves GitHub’s current default branch and inspects
its latest SHA on every click. Recovery retries the frozen SHA from the original
request. For an explicit commit, open **Status** and enter the full lowercase SHA
under **Inspect a specific commit**. Open the matching result’s details from
**Inspection history** to see policy and lock digests, locked tools, public trust
key IDs and corrective findings. Reload after an interrupted
request and choose **Recover inspection** or **Retry same inspection**.

Static inspection never enables preparation or starts work. Full adapter discovery
remains pending; signer admission and independent runtime verification are separate
preparation gates. A static inspection result alone establishes neither.

### Runtime signing and preparation

Configuration → Runtime signing supports server-generated or imported immutable
signing keys and advanced administrator-owned Secret references. The entered Name
is the actual trust-policy key name (`keyId` in the API); it is paired with the
public key. Correcting a name creates a new public configuration revision and
preserves old receipts and frozen preparation inputs. Exact setup,
updates and original receipt recovery survive reloads. Public identities and
references do not by themselves grant builds or establish runtime readiness.

The repository’s **ayni** tab keeps current status, issues and the next action together. Choose **Configure runtime** after a successful check to review the checked commit and configuration. Current progress and recovery controls stay visible; previous preparations and inspections are collapsed by default. Preparation retains frozen source/configuration, explicit build
and replacement grants, stage history, independent verification, cleanup and exact
command recovery. Runtime preparation needs an admitted deployment profile. See
[the user guide](../../docs/site/repositories.md) for setup and recovery; verified
artifact selection for project tasks remains planned.

Repository onboarding uses StartRepositoryRuntime/GetRepositoryRuntime and an
immutable receipt API. Reviewed settings are retained server-side. The leased
control-plane workflow freezes the current default-branch SHA, inspects it and
starts canonical preparation without browser orchestration. The runtime status
tracks that run, exposes deployment blockers, and links the repository package and
confirmed immutable image. Prepare latest commit reuses reviewed settings for one
new bounded run. Opening a page never renews a build grant.

### Application structure and saving

All product pages live beneath `src/routes/(app)`. Its layout owns workspace
loading, navigation, breadcrumbs and the application shell; the root layout owns
only global styles. API routes remain outside the application group. Settings
pages use `PageHeader`, `PageContent` and shared field/card components.

Workspace and project IDs belong in URL path segments. `src/lib/paths.ts` builds
encoded scoped links and pagination queries. Old query-based bookmarks redirect
to the canonical route. Missing resources return 404; unavailable services return
503 rather than an empty workspace. A project is resolved only within its URL's
workspace, including paginated collections.

Names and project settings save automatically after a short debounce, with a
shared saving/unsaved/saved status and per-field acknowledgement. Instructions
and budgets can be saved before configuring a model. A provider change waits for
a validated connection and discovered model. Edits during a save are serialized
into another revision. Unsaved edits are protected during navigation; uncertain
submitted commands retain their exact non-secret request for explicit recovery
across reloads. Known rejected updates remain editable and expose reload guidance.
Credential forms require explicit submission and keep private material out of
browser storage. New-project recovery is scoped to its workspace.
