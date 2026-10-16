# Operator onboarding

Integrations hold provider credentials; repository runtimes select those integrations.
Projects use shared repository runtimes. Changes to a runtime affect future admissions in every consuming project,
while existing tasks retain frozen inputs. See the [product model](ayni-product-flow.md).

Start the control plane without a GitHub App, registry or model credentials to
configure connections through the trusted operator UI. Model credentials are
required when admitting model-backed work, not when opening onboarding.

The sidebar exposes **Workspaces**, **Repositories**, and **Config**. Config
contains Source control, Container registries and Ayni; certificates are reached
from the Ayni overview. Projects are created and opened inside a workspace. Workspace views separate
Projects, Repositories, and Settings; repository onboarding remains in the global
Repositories catalog. These navigation groups do not change credential ownership.

Under **Config → Source control → GitHub**, create a customer-owned App through the
manifest flow or import its App ID and PEM key. The server stores credentials in
a Kubernetes Secret and returns safe connection status. Install the App on the
repositories it may access in GitHub, then connect those repositories in Ayni.
App ownership and installation permissions remain under GitHub administration.

Under **Config → Container registries → GitHub**, add a GHCR connection with an approved
image namespace, username, push token and a distinct pull-only token. Check the
independent authentication, push, pull and cleanup results. A cleanup failure is
reported separately and can be retried. Select the connection in the runtime’s **Edit** dialog on its repository overview. Repository
enrollment does not require a registry selection.
The **Connect repository** dialog starts with **Use Ayni** unchecked. Connecting
without it skips Ayni runtime onboarding, including background retries after a
restart. Enable it to select a ready GHCR connection on the same row; the selected
namespace appears below. You can leave **Configure later** selected and finish
the runtime destination afterward. For repositories connected without Ayni,
choose **Set up Ayni** on the repository page to enable onboarding later.
Ayni and its official builder always use the version in the repository’s committed
`.ayni.lock`; version upgrades require regenerating and committing that lock.

Administrator-managed connections show **Managed by your administrator** and
cannot be edited or deleted through the UI. Register them explicitly using the
Helm connection references described in the [deployment chart](../../../deploy/charts/ayni-orchestrator/README.md).
UI-managed connections support replacement, testing, disabling and deletion when
unreferenced. Credential inputs clear after submission and read APIs never return
them. Validated replacements activate automatically; no reload button, deployment
restart or Tilt restart is required. Connection pages refresh status periodically.

The repository overview shows connection status and branch once, followed by its
runtimes. Lock fingerprints and image destinations belong to each runtime.
Onboarding status, inspected commit and actionable failures persist independently
from repository access and remain visible on its overview. Managed runtime images are published at
`ghcr.io/<owner>/<repository>:ayni-<arch>-<lock>`. Authentication usernames stay
on the server and do not determine image ownership. The connection namespace is used for connection tests;
repository and runtime dialogs preview
the image destination without a namespace input. The control plane validates and
derives the destination when saved. Registry changes invalidate that runtime’s
readiness until its new destination is verified; other runtimes are unaffected.

Connect the repository and open its runtime, even if configuration is incomplete.
To restart onboarding, open **Edit → Remove repository** on its repository page
and enter the repository name to confirm. Removal clears the local connection,
snapshots, runtime profiles, revisions, and artifact records. Reconnect from
Repositories to start with fresh onboarding and runtime selections. GitHub content,
registry images, provider integrations, and signing certificates remain intact.
Project references (including archived projects), retained tasks, and active
runtime preparation block removal. Failed preparation retains durable cleanup
intents; the runtime reconciler retries cleanup of its owned jobs and temporary
resources, including after a restart. Interrupted onboarding intents are retired
so recovery cannot recreate the removed connection.

Select a ready certificate and GHCR connection. Analyze repository reports missing
committed policy or lock, incompatible configuration, trust, registry and platform
findings with corrective actions. Review and merge any certificate trust PR.
General configuration bootstrap remains owner-managed.

Once prerequisites pass, the control plane independently verifies an existing
image or automatically builds, certifies, publishes and verifies a missing one.
All successful builds publish. Mismatches offer Rebuild; access failures offer
retry after repair. Rebuild cannot bypass setup. Add the repository to a project
and submit work once the runtime is Ready. Repository environment-building Actions
are removed through separate reviewed PRs; ordinary CI and deployment-component
packaging remain. The control plane installs no workflows or checks.

Submit a task against a ready runtime. Admission freezes source revision,
runtime digest, model, constraints, coding turns, quality repair limit and
deadline. The worker codes, commits, checks the full repository and makes bounded
repairs before creating a draft with the actual quality result.

Task history shows status and phase. Task details show Progress, Quality and
Pull request, including turns, round, frozen limits, completion counts, signal
details and actionable findings. Unavailable measurements remain unavailable.
A failed-quality draft is completed delivery. Publication failure can be retried
against retained final work without coding again. Ordinary retry creates a new
attempt.

Before a breaking deployment, follow the exact-resource inventory and migration
procedure in [operations](../operations/operations.md). Existing source repositories,
runtime images and GitHub pull requests are retained.

Manage the GitHub App at `/source-control/github`. Use **Add GitHub App** to create an App in your organization or personal account, or import an existing App ID and private key. The compact connection card opens an **Edit** dialog for key rotation, testing, disablement, and deletion; administrator-managed Apps expose read-only **Details**. App installation stays in GitHub. Legacy `/setup/github` links redirect here, and existing Apps using `/setup/github/complete` retain callback compatibility.

### Project navigation and settings

Project pages use `?view=overview`, `?view=plans`, and `?view=tasks`. View links
preserve the selected `conversation` and discard task pagination. Plans accept
`version` for a retained version link. An unavailable conversation or version
shows recovery feedback instead of silently selecting a different record.

The Conversation tab retains `view=overview` links. Without a selected
`conversation`, it shows a blank composer; the first message creates the
conversation and sends that message with separate stable idempotency keys.
Titles are derived from the message and limited to 30 Unicode code points.
Enter sends a message and Shift+Enter inserts a newline.

The Conversation tab keeps the timeline in a fixed-height keyboard-scrollable region above
the plan. Session details, saved context, messages, and research activity scroll
within it. The composer remains outside the timeline. Conversation invalidations
and periodic refresh run across all three views, preserving a draft message and
an explicitly selected plan version during refresh.

Workspace shared configuration owns membership, agent defaults, and planner idle
defaults. The project settings dialog edits its goal, selected repositories, and
model overrides. The goal also carries project planning instructions; there is no
separate custom instruction field. Repository instruction files remain in source
control. Conversation session settings override only that conversation's idle
timeout. Mutations retain their resource-version or update-cursor guards.

Plan review uses the existing safe BFF projection of run identity, model, calls,
and token usage. Credentials stay server-side. Linked task progress is loaded
independently of the task-list page so retained plan results remain inspectable
when paging tasks. No plan, approval, or task-admission contract changes are
required for these views.

### Task instructions and budgets

Task creation selects one project repository, its ready runtime, and a provider-available
model from `gpt-5.6-luna`, `gpt-5.6-terra`, and `gpt-5.6-sol`. The request describes
the work; acceptance criteria describe observable outcomes and guide the agent rather
than replacing mandatory Ayni checks. Protected paths use exact repository-relative
files or directory prefixes, not globs. Ayni policy and lock files remain protected.
Test edits are enabled by default and can be disabled for a task.

The Agent iterations field defaults to 24 and accepts 1–128 model turns across
implementation and quality repairs. The selected value is frozen into the task and
used by the worker, independently of subsequent deployment-default changes. API
clients that omit `coding_turns` retain the deployment default. The deadline defaults
to one hour from submission and includes queueing, coding, checks, and publication.
